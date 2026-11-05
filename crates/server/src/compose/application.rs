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
use moatless_tasks::secret_request::service::SecretRequests;
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
            task_repositories,
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
            review_guides,
            artifacts,
            memories,
            records,
            repositories,
            repository_reader,
            workspaces,
            workspace_resolver,
            skill_fetcher,
            plugin_sync,
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
            message_buffer_worker,
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
            codex_http,
        );
        let credential_sync = task_graph.credential_sync(credentials.clone());

        let git_sync = moatless_workspaces::compose::git_sync(
            pool.clone(),
            git_host_tokens.clone(),
            config.http.git_content,
            config.github_api_base.clone(),
            config.workspace_sync.identity_grants.clone(),
        );

        let loop_sync = loop_graph.loop_sync(
            connection_routing.clone(),
            tag_resolver.clone(),
            workspace_resolver.clone(),
            git_host_tokens.clone(),
            config.http.git_content,
            config.github_api_base.clone(),
        );

        // Hoisted so the self-echo filter and the struct literal share one cache.
        let recently_sent = Arc::new(RecentlySent::new());

        let skills = moatless_workspaces::compose::skills(
            pool.clone(),
            git_host_tokens.clone(),
            skill_fetcher,
            plugin_sync.clone(),
            std::time::Duration::from_secs(config.skill.cache_ttl_secs),
        );

        // Wired once here so every task-creation path (HTTP, loops, adapters)
        // runs the same check, via the `PreflightPort` on `AppState` rather
        // than each caller reassembling it.
        let preflight: Arc<dyn PreflightPort> = Arc::new(RealPreflight::new(
            PreflightCredentials {
                git_host_tokens: git_host_tokens.clone(),
                users: user_directory.clone(),
                workspaces: workspace_resolver.clone(),
                credential_sync: credential_sync.clone(),
                workspace_credentials: workspace_credentials.clone(),
            },
            Arc::new(RealGitHostValidator {
                github_api_base: config.github_api_base.clone(),
                github: config.http.github,
                self_hosted_git: config.http.self_hosted_git,
            }),
            ports.sandbox_graph.sandbox_ingress(),
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
            task_switch,
        } = task_graph.assemble(TaskWorkflowInputs {
            repository_reader: repository_reader.clone(),
            workspace_resolver: workspace_resolver.clone(),
            user_display: user_directory.clone(),
            users: user_directory.clone(),
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
            Arc::new(ProviderGitHostTokenValidator::new(
                config.github_api_base.clone(),
                config.http.github,
            )),
            config.auth.github_app_oauth().is_some(),
        );
        let sandbox_callers = Arc::new(crate::proxy::caller::SandboxCallers::new(
            ports.sandbox_graph.sandbox_ingress(),
            ports.sandbox_graph.token_generations(),
            config.sandbox.task_token_secret.clone(),
        ));
        let git_proxy = Arc::new(crate::git_proxy::GitProxy::new(
            sandbox_callers.clone(),
            workspace_resolver.clone(),
            repository_reader.clone(),
            user_directory.clone(),
            git_host_tokens.clone(),
            config.http.git_proxy,
        ));
        let github_proxy = Arc::new(crate::github_proxy::GitHubProxy::new(
            sandbox_callers.clone(),
            workspace_resolver.clone(),
            workspace_credentials.clone(),
            git_host_tokens.clone(),
            &config.github_api_base,
            crate::github_proxy::GITHUB_UPLOADS_BASE,
            config.http.github_proxy,
        ));
        let service_proxy = Arc::new(crate::service_proxy::ServiceProxyGateway::new(
            sandbox_callers.clone(),
            ports.service_access.clone(),
            workspace_resolver.clone(),
            workspace_credentials.clone(),
            secrets.clone(),
            config.http.github_proxy,
        ));
        let model_gateway = Arc::new(crate::model_gateway::ModelGateway::new(
            sandbox_callers,
            workspace_resolver.clone(),
            workspace_credentials.clone(),
            secrets.clone(),
            harness_credentials.clone(),
            credential_sync.clone(),
            config.model_gateway.clone(),
            crate::model_gateway::ProviderBases::default(),
            config.http.model_gateway,
        ));
        let agent_harness_access = Arc::new(AgentHarnessAccess::new(
            credentials.clone(),
            codex_oauth_config.clone(),
            Arc::new(ProviderCodexDeviceLogin::new(config.http.codex)),
            user_directory.clone(),
        ));
        let app_administration = Arc::new(AppAdministration::new(
            github_apps.clone(),
            user_directory.clone(),
            Arc::new(ProviderGitHubAppProver {
                github_api_base: config.github_api_base.clone(),
                github: config.http.github,
            }),
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
        let loop_task_opener: Arc<dyn moatless_ports::task::TaskOpener> =
            Arc::new(moatless_loops::rooms::RoomOpeningTaskOpener::new(
                task_creation.clone(),
                adapter_delivery.clone(),
            ));
        let routing = loop_graph.routing(RoutingInputs {
            adapter_connections: connection_routing.clone(),
            messages: transcript.clone(),
            tasks: task_reader.clone(),
            task_creation: loop_task_opener.clone(),
            message_send: message_send.clone(),
            sandbox_waker: sandbox_waker.clone(),
            adapters: adapters.clone(),
            message_buffer: config.message_buffer.clone(),
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
            notice_replies: Arc::new(moatless_tasks::artifact::notice_reply::NoticeReplies::new(
                tasks.clone(),
                artifacts.clone(),
            )),
        });

        let adapter_commands = Arc::new(crate::adapters::operations::AdapterCommands {
            adapter_connections: adapter_connections.clone(),
            adapters: adapters.clone(),
            task_reader: task_reader.clone(),
            files: files.clone(),
            external_reply: external_reply.clone(),
            task_bindings: task_bindings.clone(),
            users: user_directory.clone(),
            room_admission: adapter_delivery.clone(),
        });

        let secret_requests = Arc::new(SecretRequests::new(
            pool.clone(),
            tasks.clone(),
            secrets.clone(),
            message_send.clone(),
        ));

        Self {
            adapter_commands,
            secret_requests,
            tasks,
            task_reader,
            task_creation,
            loop_task_opener,
            task_fork,
            task_rewind,
            task_switch,
            preflight,
            relation_recorder: task_relations.clone(),
            task_relations,
            task_repositories,
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
            review_guides,
            artifacts,
            memories,
            records,
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
            message_buffer_worker,
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
            github_proxy,
            service_proxy,
            model_gateway,
            agent_harness_access,
            app_administration,
            recently_sent,
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
