use std::sync::Arc;
use tracing::info;

use crate::compose::Ports;
use crate::sandbox::grpc_server::BackendSandboxServiceImpl;
use crate::sandbox::k8s::client::K8sClient;
use moatless_core::config::Config;
use moatless_core::db::{self, DbPool};
use moatless_core::events::{EventPublisher, FeedEventPublisher};
use moatless_identity::secret::service::LocalEnvelopeSecretCipher;
use moatless_workspaces::skill::fetcher::GitHostSkillFetcher;

pub async fn init_database(
    database_url: Option<&str>,
    max_connections: u32,
    run_migrations: bool,
) -> Result<Option<DbPool>, Box<dyn std::error::Error>> {
    let url = if let Some(u) = database_url {
        u
    } else {
        info!("DATABASE_URL not set — database features disabled");
        return Ok(None);
    };

    const MAX_ATTEMPTS: u32 = 10;
    for attempt in 1..=MAX_ATTEMPTS {
        match db::create_pool(url, max_connections).await {
            Ok(pool) => {
                db::record_pool_metrics(&pool);
                info!("Database pool initialized");
                if run_migrations {
                    if let Err(e) = db::run_migrations(&pool).await {
                        tracing::error!(error = %e, "Database migration failed");
                        return Err(e.into());
                    }
                    info!("Database migrations complete");
                } else {
                    info!("RUN_MIGRATIONS=false — skipping database migrations");
                }
                return Ok(Some(pool));
            }
            Err(e) if attempt < MAX_ATTEMPTS => {
                tracing::warn!(
                    error = %e,
                    attempt,
                    max_attempts = MAX_ATTEMPTS,
                    "Database connection failed, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    attempts = MAX_ATTEMPTS,
                    "Failed to create database pool after retries — database features disabled"
                );
                return Ok(None);
            }
        }
    }
    Ok(None)
}

/// The event publisher, which is the database — `None` only where there is no
/// database at all, which is also where nothing consumes events.
pub fn init_event_publisher(pool: Option<&DbPool>) -> Option<Arc<dyn EventPublisher>> {
    let Some(pool) = pool else {
        info!("DATABASE_URL not set — event publishing disabled");
        return None;
    };
    Some(Arc::new(FeedEventPublisher::new(pool.clone())))
}

pub async fn init_s3(config: &Config) -> Option<moatless_s3::S3Storage> {
    if config.s3.bucket_name.is_none() {
        info!("S3_BUCKET_NAME not set — file uploads disabled");
        return None;
    }
    let storage = moatless_s3::S3Storage::from_default_chain(config.s3.region.clone()).await;
    info!("S3 client initialized");
    Some(storage)
}

/// What the workspace-repository grant assumes its role through.
pub async fn init_sts(config: &Config) -> Option<moatless_s3::RoleAssumer> {
    _ = config.sandbox.workspace_repository_role_arn.as_ref()?;
    let assumer = moatless_s3::RoleAssumer::from_default_chain(config.s3.region.clone()).await;
    info!("STS client initialized for workspace repository grants");
    Some(assumer)
}

pub fn build_k8s_sandbox_adapter(
    config: &Config,
    k8s: K8sClient,
    s3: Option<Arc<moatless_s3::S3Storage>>,
    plan: Option<crate::sandbox::lifecycle::ProvisionDeps>,
) -> Arc<crate::sandbox::k8s::provider::K8sSandboxAdapter> {
    use crate::sandbox::k8s::provider::K8sSandboxAdapter;
    use crate::sandbox::k8s::status_resolver::StatusResolver;

    let status_resolver = StatusResolver::new(k8s.clone());

    Arc::new(K8sSandboxAdapter {
        config: Arc::new(config.clone()),
        k8s,
        status_resolver,
        plan,
        s3,
    })
}

pub fn build_docker_sandbox_adapter(
    config: &Config,
    client: crate::sandbox::docker::client::DockerClient,
    s3: Option<Arc<moatless_s3::S3Storage>>,
    plan: Option<crate::sandbox::lifecycle::ProvisionDeps>,
) -> Arc<crate::sandbox::docker::provider::DockerSandboxAdapter> {
    use crate::sandbox::docker::provider::DockerSandboxAdapter;
    Arc::new(DockerSandboxAdapter::new(
        Arc::new(config.clone()),
        client,
        plan,
        s3,
    ))
}

pub fn build_local_sandbox_adapter(
    config: &Config,
    client: crate::sandbox::local::client::LocalClient,
    plan: Option<crate::sandbox::lifecycle::ProvisionDeps>,
) -> Arc<crate::sandbox::local::provider::LocalSandboxAdapter> {
    use crate::sandbox::local::provider::LocalSandboxAdapter;
    Arc::new(LocalSandboxAdapter::new(
        Arc::new(config.clone()),
        client,
        plan,
    ))
}

pub fn build_merged_openapi() -> utoipa::openapi::OpenApi {
    use crate::access_settings::routes::AccessSettingsApiDoc;
    use crate::adapters::routes::AdapterApiDoc;
    use crate::feature_flags::FeatureFlagsApiDoc;
    use crate::file_storage::routes::FileStorageApiDoc;
    use crate::repository_sync::routes::RepositorySyncApiDoc;
    use crate::sandbox::routes::SandboxApiDoc;
    use moatless_decisions::routes::DecisionApiDoc;
    use moatless_identity::auth::routes::AuthApiDoc;
    use moatless_identity::secret::routes::SecretApiDoc;
    use moatless_identity::team::routes::TeamApiDoc;
    use moatless_loops::loops::http::LoopsApiDoc;
    use moatless_tasks::agent::routes::AgentApiDoc;
    use moatless_tasks::artifact::routes::ArtifactApiDoc;
    use moatless_tasks::collection::routes::CollectionApiDoc;
    use moatless_tasks::feedback::routes::FeedbackApiDoc;
    use moatless_tasks::memory::routes::MemoryApiDoc;
    use moatless_tasks::message::routes::MessageApiDoc;
    use moatless_tasks::review_guide::routes::ReviewGuideApiDoc;
    use moatless_tasks::tag::routes::TagApiDoc;
    use moatless_tasks::task::routes::TaskApiDoc;
    use moatless_tasks::task_event::routes::TaskEventApiDoc;
    use moatless_workspaces::plugin::routes::PluginApiDoc;
    use moatless_workspaces::repository::routes::RepositoryApiDoc;
    use moatless_workspaces::skill::routes::SkillApiDoc;
    use moatless_workspaces::workspace::routes::WorkspaceApiDoc;
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(
        components(schemas(
            // Event types + sub-types
            moatless_types::event::EventType,
            moatless_types::sandbox::SandboxStatus,
            moatless_types::agent::AgentSubType,
            moatless_types::task::TaskSubType,
            moatless_types::task::TaskTurnSubType,
            // Event payloads
            moatless_types::sandbox::SandboxEventPayload,
            moatless_types::agent::AgentEventPayload,
            moatless_types::task::TaskEventPayload,
            moatless_types::task::TaskTurnEventPayload,
            moatless_types::sandbox::ResourcesEventPayload,
            moatless_types::event::StreamEvent,
            // Claude SDK types
            claude_agent_sdk::api::content::ContentBlock,
            claude_agent_sdk::api::content::ImageSource,
            claude_agent_sdk::api::content::ImageSourceType,
            claude_agent_sdk::api::content::ApiMessage,
            claude_agent_sdk::api::content::Usage,
            claude_agent_sdk::api::content::ModelUsage,
            claude_agent_sdk::api::content::PermissionDenial,
            claude_agent_sdk::api::messages::AssistantMessage,
            claude_agent_sdk::api::messages::AssistantMessageError,
            claude_agent_sdk::api::messages::ResultSuccess,
            claude_agent_sdk::api::messages::ResultError,
            // User-input types
            moatless_tasks::message::schemas::QuestionOption,
            moatless_tasks::message::schemas::Question,
            moatless_tasks::message::schemas::AllowedPrompt,
            moatless_tasks::message::schemas::AskUserQuestionInput,
            moatless_tasks::message::schemas::ExitPlanModeInput,
            crate::sandbox::schemas::MoatlessToolName,
        ))
    )]
    struct EventsApiDoc;

    let mut merged = SandboxApiDoc::openapi();
    merged.merge(AgentApiDoc::openapi());
    merged.merge(AuthApiDoc::openapi());
    merged.merge(TaskApiDoc::openapi());
    merged.merge(MessageApiDoc::openapi());
    merged.merge(FeedbackApiDoc::openapi());
    merged.merge(ArtifactApiDoc::openapi());
    merged.merge(MemoryApiDoc::openapi());
    merged.merge(CollectionApiDoc::openapi());
    merged.merge(ReviewGuideApiDoc::openapi());
    merged.merge(FileStorageApiDoc::openapi());
    merged.merge(RepositoryApiDoc::openapi());
    merged.merge(RepositorySyncApiDoc::openapi());
    merged.merge(SecretApiDoc::openapi());
    merged.merge(TagApiDoc::openapi());
    merged.merge(AccessSettingsApiDoc::openapi());
    merged.merge(AdapterApiDoc::openapi());
    merged.merge(LoopsApiDoc::openapi());
    merged.merge(TeamApiDoc::openapi());
    merged.merge(DecisionApiDoc::openapi());
    merged.merge(EventsApiDoc::openapi());
    merged.merge(TaskEventApiDoc::openapi());
    merged.merge(FeatureFlagsApiDoc::openapi());
    merged.merge(PluginApiDoc::openapi());
    merged.merge(SkillApiDoc::openapi());
    merged.merge(WorkspaceApiDoc::openapi());

    merged.info = utoipa::openapi::InfoBuilder::new()
        .title("Moatless Vibe Backend API")
        .version(env!("CARGO_PKG_VERSION"))
        .description(Some(
            "Sandbox orchestration and authentication for Moatless Vibe",
        ))
        .build();

    merged
}

pub fn grpc_service(
    config: &Config,
    ports: Option<&Ports>,
    event_publisher: Option<Arc<dyn EventPublisher>>,
    s3: Option<Arc<moatless_s3::S3Storage>>,
    workspace_repository: Option<
        Arc<crate::sandbox::workspace_repository::WorkspaceRepositoryAccess>,
    >,
    connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
) -> BackendSandboxServiceImpl {
    // `tasks` and `users` are read by the authorizer's control gate; the request
    // handlers reach them (and the rest of the db-backed set) through `handlers`.
    let tasks = ports.map(|p| p.tasks.clone());
    let users = ports
        .map(|p| p.user_directory.clone() as Arc<dyn moatless_ports::identity::CommitIdentity>);
    let backend_config = Arc::new(config.clone());
    let token_secret = config.sandbox.task_token_secret.clone();
    let s3_bucket = config.s3.bucket_name.clone();
    let s3_key_prefix = config.s3.key_prefix.clone();
    let transcript_s3_prefix = config.s3.transcript_prefix.clone();
    let workspace_s3_prefix = config.s3.workspace_prefix.clone();
    // Without a database there are no stored Apps to reach, so the deployment's
    // login App is the whole registry.
    let github_apps = match ports {
        Some(ports) => ports.github_apps.clone(),
        None => moatless_identity::compose::github_app_registry(
            config,
            None,
            LocalEnvelopeSecretCipher::from_config(&config.secrets),
        ),
    };
    let github_app_service = github_apps.default_service();

    // The db-backed ports resolve as two present-together values off the one
    // `ports` gate: the durable-admission writers and the handler set. Both are
    // `None` databaseless.
    let ingress = ports.map(|p| crate::sandbox::grpc_server::IngressPorts {
        admission: p.task_graph.event_ingress(event_publisher.clone()),
        task_turns: p.task_turns.clone(),
        task_events: p.task_events.clone(),
    });
    let handlers = ports.map(|p| {
        // Tasks-band orchestrator — guard + fan-out over the pure service.
        let credential_sync = p.task_graph.credential_sync(
            p.credentials.clone(),
            Arc::new(crate::sandbox::notify::CredentialBroadcastService::new(
                Arc::clone(&connector),
            )),
        );
        crate::sandbox::grpc_server::HandlerDbPorts {
            messages: p.messages.clone(),
            transcript: p.transcript.clone(),
            workspace_resolver: p.workspace_resolver.clone(),
            tasks: p.tasks.clone(),
            task_relations: p.task_relations.clone(),
            memories: p.memories.clone(),
            task_forks: p.task_forks.clone(),
            secrets: p.secrets.clone(),
            credential_sync,
            sandboxes: p.sandbox_graph.sandbox_ingress(),
            workspace_credentials: p.workspace_credentials.clone(),
            git_host_tokens: p.git_host_tokens.clone(),
            files: p.files.clone(),
            provider_link: p.user_directory.clone(),
            user_logins: p.user_directory.clone(),
            commit_identity: p.user_directory.clone(),
            harness_credentials: p.harness_credentials.clone(),
            trajectory: p.task_graph.trajectory(),
            tags: p.tags.clone(),
            plugin_archives: moatless_workspaces::compose::plugin_archives(
                Arc::new(GitHostSkillFetcher::new(config.http.plugin_archive)),
                p.git_host_tokens.clone(),
            ),
        }
    });

    // The single verification layer. `Some(secret)` → enforcing, `None` → the
    // dev bypass (production refuses to boot without the secret; see
    // `Config::validate`).
    let authorizer = crate::sandbox::grpc_server::SandboxAuthorizer::from_config(
        token_secret.clone(),
        config.sandbox.accept_claimless_tokens,
        tasks.clone(),
        users.clone(),
        ports.map(|p| {
            p.sandbox_graph.token_generations()
                as Arc<dyn moatless_core::sandbox_jwt::TokenGenerations>
        }),
    );

    BackendSandboxServiceImpl {
        events: event_publisher,
        handlers,
        ingress,
        task_token_secret: token_secret,
        authorizer,
        github_app_service,
        github_apps,
        backend_config,
        s3,
        workspace_repository,
        s3_bucket,
        s3_key_prefix,
        transcript_s3_prefix,
        workspace_s3_prefix,
        event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::new()),
        admissions: tokio_util::task::TaskTracker::new(),
    }
}
