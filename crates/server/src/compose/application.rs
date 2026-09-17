//! Assemble application services over the process ports.

use crate::adapter_installations::AdapterInstallations;
use crate::adapters::self_echo::SelfEchoFilter;
use crate::agent_harness_access::{AgentHarnessAccess, ProviderCodexDeviceLogin};
use crate::app_administration::ProviderGitHubAppProver;
use crate::compose::Ports;
use crate::sandbox::join::ConnectorSandboxJoiner;
use crate::sandbox::lifecycle::{Lifecycle, NotifyPort};
use crate::sandbox::preflight::{PreflightCredentials, RealGitHostValidator, RealPreflight};
use crate::state::AppState;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_identity::adapter_apps::administration::AppAdministration;
use moatless_identity::git_host_access::ProviderGitHostTokenValidator;
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_loops::compose::{ExternalReplyInputs, OutboundSenderInputs, RoutingInputs};
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_ports::sandbox::{PreflightPort, SandboxJoiner};
use moatless_tasks::compose::{TaskOperations, TaskWorkflowInputs};
use moatless_workspaces::skill::fetcher::GitHostSkillFetcher;
use moatless_workspaces::skill::fetcher::SkillFetcher;
use sandbox_proto::client::SandboxConnector;
use std::sync::Arc;

impl AppState {
    /// Build `AppState` over the ports the composition root assembled.
    ///
    /// The services wired here need a runtime handle the composition root lacks
    /// — the sandbox connector and lifecycle handle — so they're assembled from
    /// `ports` rather than in it.
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
        s3: Option<moatless_s3::S3Storage>,
        adapter_installations: Option<Arc<AdapterInstallations>>,
    ) -> Self {
        let Ports {
            pool,
            tasks,
            task_reader,
            task_relations,
            task_graph,
            loop_graph,
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
            plugins,
            loops,
            loop_reader,
            adapter_connections,
            connection_routing,
            adapter_delivery,
            adapter_delivery_worker,
            user_directory,
            credentials,
            codex_http,
            harness_credentials,
            adapter_apps,
            secrets,
            github_apps,
            github_app_service,
            git_host_tokens,
            forgejo_instances,
            forgejo_access,
            token_verifier,
            ..
        } = ports.clone();

        // The concrete issuer is built once in `compose`; every consumer below
        // takes the port.
        let git_host_tokens: Arc<dyn GitHostTokenIssuer> = git_host_tokens;

        let adapters = adapter_installations
            .as_ref()
            .map(|installations| installations.adapters());

        let repository_branches = moatless_workspaces::compose::repository_branches(
            pool.clone(),
            git_host_tokens.clone(),
            credentials.clone(),
            codex_http,
        );
        // Tasks-band orchestrator: the in-flight guard + sandbox fan-out over
        // the pure credential service.
        let credential_sync = task_graph.credential_sync(
            credentials.clone(),
            Arc::new(crate::sandbox::notify::CredentialBroadcastService::new(
                connector.clone(),
            )),
        );

        let git_sync =
            moatless_workspaces::compose::git_sync(pool.clone(), git_host_tokens.clone());

        let loop_sync = loop_graph.loop_sync(
            connection_routing.clone(),
            tag_resolver.clone(),
            workspace_resolver.clone(),
            git_host_tokens.clone(),
        );

        // Hoisted so the self-echo filter and the struct literal share one cache.
        let recently_sent = Arc::new(RecentlySent::new());

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
        let skill_fetcher: Arc<dyn SkillFetcher> = Arc::new(GitHostSkillFetcher::new(skill_http));
        let plugin_sync = moatless_workspaces::compose::plugin_sync(
            pool.clone(),
            skill_fetcher.clone(),
            git_host_tokens.clone(),
        );
        let skills = moatless_workspaces::compose::skills(
            pool.clone(),
            git_host_tokens.clone(),
            skill_fetcher,
            plugin_sync.clone(),
            std::time::Duration::from_secs(config.skill.cache_ttl_secs),
        );

        // Wire the git-host credential preflight once here so every task-creation path
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
            Arc::new(RealGitHostValidator),
        ));

        let sandbox_joiner: Arc<dyn SandboxJoiner> =
            Arc::new(ConnectorSandboxJoiner::new(connector.clone()));

        // Durable admission is taken off the graph before `assemble` consumes
        // it; the relay `app.rs` spawns is composed from the same handle.
        let event_ingress = task_graph.event_ingress(events.clone());

        // The tasks band assembles its own workflow services over the stores it
        // owns; this process supplies the foreign ports they read.
        let TaskOperations {
            task_creation,
            message_send,
            task_closer,
            task_presenter,
            task_fork,
            task_rewind,
        } = task_graph.assemble(TaskWorkflowInputs {
            repository_reader: repository_reader.clone(),
            workspace_resolver: workspace_resolver.clone(),
            user_display: user_directory.clone(),
            task_bindings: binding_directory.clone(),
            sandbox: lifecycle.clone(),
            starter: lifecycle.clone(),
            joiner: sandbox_joiner,
            preflight: preflight.clone(),
            events: events.clone(),
            files: files.clone(),
            self_echo: Arc::new(SelfEchoFilter::new(recently_sent.clone(), adapters.clone())),
            config: config.clone(),
        });

        // Config-section ports, built once so a secret-bearing section is never
        // re-cloned per request. `config` still holds the whole tree for
        // unmigrated call sites.
        let auth_config = Arc::new(config.auth.clone());
        let codex_oauth_config = Arc::new(config.codex_oauth.clone());
        let secrets_config = Arc::new(config.secrets.clone());
        let s3_config = Arc::new(config.s3.clone());

        let git_host_access = moatless_identity::compose::git_host_access(
            pool.clone(),
            secrets.clone(),
            Arc::new(ProviderGitHostTokenValidator::default()),
            config.auth.github_app_oauth().is_some(),
        );
        let git_proxy = Arc::new(crate::git_proxy::GitProxy::new(
            ports.sandbox_graph.sandbox_ingress(),
            workspace_resolver.clone(),
            repository_reader.clone(),
            user_directory.clone(),
            git_host_tokens.clone(),
            config.sandbox.task_token_secret.clone(),
        ));
        let agent_harness_access = Arc::new(AgentHarnessAccess::new(
            credentials.clone(),
            credential_sync.clone(),
            codex_oauth_config.clone(),
            Arc::new(ProviderCodexDeviceLogin::default()),
            user_directory.clone(),
        ));
        let app_administration = Arc::new(AppAdministration::new(
            github_apps.clone(),
            user_directory.clone(),
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
        // `AppState`. The loops band supplies the Binding, Loop and receipt
        // stores; this process supplies the foreign ports.
        let routing = loop_graph.routing(RoutingInputs {
            adapter_connections: connection_routing.clone(),
            messages: transcript.clone(),
            tasks: task_reader.clone(),
            task_creation: task_creation.clone(),
            message_send: message_send.clone(),
            sandbox_waker: sandbox_waker.clone(),
            adapters: adapters.clone(),
        });

        // Outbound reply delivery owns the ports its send path reads. The
        // `OutboundSender` is built here and handed to `ExternalReplyService`
        // (the SelfEchoFilter/RoutingService pattern), so neither struct's
        // constructor takes more than the ports it holds and neither names
        // `AppState`.
        let outbound_sender = loop_graph.outbound_sender(OutboundSenderInputs {
            adapter_connections: connection_routing.clone(),
            tasks: task_reader.clone(),
            recently_sent: recently_sent.clone(),
            messages: transcript.clone(),
            task_turns: turn_counter.clone(),
            auth_config: auth_config.clone(),
            files: files.clone(),
            workspaces: workspace_resolver.clone(),
            users: user_directory.clone(),
        });
        let external_reply = loop_graph.external_reply(ExternalReplyInputs {
            adapters: adapters.clone(),
            adapter_connections: adapter_connections.clone(),
            connection_routing: connection_routing.clone(),
            messages: transcript.clone(),
            tasks: task_reader.clone(),
            recently_sent: recently_sent.clone(),
            task_turns: turn_counter.clone(),
            outbound: outbound_sender,
        });

        let adapter_commands = Arc::new(crate::adapters::operations::AdapterCommands {
            adapter_connections: adapter_connections.clone(),
            adapters: adapters.clone(),
            task_reader: task_reader.clone(),
            files: files.clone(),
            external_reply: external_reply.clone(),
            task_bindings: task_bindings.clone(),
            users: user_directory.clone(),
        });

        Self {
            adapter_commands,
            tasks,
            task_reader,
            task_creation,
            task_fork,
            task_rewind,
            preflight,
            relation_recorder: task_relations.clone(),
            task_relations,
            task_bindings,
            binding_directory,
            adapter_connections,
            connection_routing,
            routing,
            external_reply,
            loops,
            loop_reader,
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
            user_directory,
            credentials,
            harness_credentials,
            adapter_apps,
            secrets,
            task_events,
            event_ingress,
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
            adapter_delivery_worker,
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
            forgejo_access,
            forgejo_instances,
            git_proxy,
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
