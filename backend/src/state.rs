//! Unified application state for API routes.
//!
//! All handler-facing database access goes through repository trait objects,
//! enabling full unit-test mocking without a real database.

use std::sync::Arc;

use crate::adapters::linear::active_turns::ActiveLinearTurns;
use crate::adapters::Adapters;
use crate::auth::github_app_registry::GitHubAppRegistry;
use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use crate::auth::provider_tokens::{PgProviderTokenAuthRepository, ProviderTokenAuthRepository};
use crate::auth::users::{PgUserRepository, UserRepository};
use crate::bindings::recently_sent::RecentlySent;
use crate::command::dao::{CommandRepository, PgCommandRepository};
use crate::config::Config;
use crate::credentials::{BroadcastContext, CredentialService};
use crate::db::DbPool;
use crate::events::EventPublisher;
use crate::feedback::dao::{FeedbackRepository, PgFeedbackRepository};
use crate::figma::dao::{FigmaRepository, PgFigmaRepository};
use crate::file_storage::dao::{FileRepository, PgFileRepository};
use crate::message::dao::{MessageRepository, PgMessageRepository};
use crate::plugin_repo::dao::{PgPluginRepoRepository, PluginRepoRepository};
use crate::repository::dao::{PgRepoRepository, RepoRepository};
use crate::sandbox::provider::SandboxProvider;
use crate::secret::audit::{PgSecretAuditRepository, SecretAuditRepository};
use crate::secret::dao::{PgSecretRepository, SecretRepository};
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};
use crate::skill::fetcher::{GitHubSkillFetcher, SkillFetcher};
use crate::skill::service::SkillService;
use crate::tag::dao::{PgTagRepository, TagRepository};
use crate::task::dao::{PgTaskRepository, TaskRepository};
use crate::task::relations::{PgTaskRelationRepository, TaskRelationRepository};
use crate::task_event::dao::{PgTaskEventRepository, TaskEventRepository};
use crate::task_template::dao::{PgTaskTemplateRepository, TaskTemplateRepository};
use crate::team::dao::{PgTeamRepository, TeamRepository};

/// Shared application state for task/message/agent/command/file routes.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd).
/// Repository trait objects enable handler unit tests without Postgres.
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool — kept for auth routes and infrastructure
    /// code that constructs its own state (gRPC server, idle reaper, etc.).
    pub db: DbPool,
    /// Immutable configuration shared across handlers.
    pub config: Arc<Config>,
    /// Sandbox workspace provider (trait object for K8s or future backends).
    pub sandbox: Arc<dyn SandboxProvider>,
    /// Optional event publisher for real-time event streaming (trait object).
    pub events: Option<Arc<dyn EventPublisher>>,
    /// Optional S3 client for file uploads/downloads.
    pub s3: Option<aws_sdk_s3::Client>,
    /// Optional external adapters.
    pub adapters: Option<Arc<Adapters>>,
    /// Optional GitHub App service for installation tokens and user token refresh.
    ///
    /// Convenience handle to the default (user-auth) App. Equivalent to
    /// `github_apps.default_service()`. Kept for back-compat with existing
    /// call sites; new code that needs to pick an App by key should go
    /// through `github_apps` instead.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    /// Registry of all configured GitHub Apps (the default user-auth App plus
    /// any additional bot Apps from `GITHUB_APPS_JSON`). Bot users' provider
    /// tokens reference an App by `key` (stored in `provider_user_id`).
    pub github_apps: Arc<GitHubAppRegistry>,
    /// Generic credential service — serializes refreshes per `(user, key)` and
    /// broadcasts rotations to running sandboxes.
    pub credentials_svc: Arc<CredentialService>,
    /// Short-lived in-memory cache of `(task_id, external_event_id)` we
    /// just sent outbound — closes the race where the inbound webhook lands
    /// before `set_external_event_id` commits. See [`RecentlySent`].
    pub recently_sent: Arc<RecentlySent>,
    /// Per-task gate that suppresses stray Linear activity emission after a
    /// turn has terminated. See [`ActiveLinearTurns`].
    pub linear_active_turns: Arc<ActiveLinearTurns>,
    /// Maps `tool_use_id` → bucketed tool-type label, populated when an
    /// assistant `tool_use` block is observed so the later (separate)
    /// `tool_result` event can be labelled by tool type for the
    /// `moatless_agent_tool_calls_total` metric without a DB round-trip.
    /// Bounded LRU; a `messages.find_tool_call` lookup covers cross-process /
    /// post-restart misses, falling back to `unknown`.
    pub tool_call_names: Arc<moka::future::Cache<String, &'static str>>,

    // =========================================================================
    // Repository trait objects
    // =========================================================================
    pub tasks: Arc<dyn TaskRepository>,
    pub task_relations: Arc<dyn TaskRelationRepository>,
    pub messages: Arc<dyn MessageRepository>,
    pub feedback: Arc<dyn FeedbackRepository>,
    pub repositories: Arc<dyn RepoRepository>,
    pub commands: Arc<dyn CommandRepository>,
    pub tags: Arc<dyn TagRepository>,
    pub files: Arc<dyn FileRepository>,
    pub figma_imports: Arc<dyn FigmaRepository>,
    pub users: Arc<dyn UserRepository>,
    pub provider_tokens: Arc<dyn ProviderTokenRepository>,
    pub provider_token_auth: Arc<dyn ProviderTokenAuthRepository>,
    pub secret_records: Arc<dyn SecretRepository>,
    pub secret_audits: Arc<dyn SecretAuditRepository>,
    pub secrets: Arc<SecretService>,
    pub teams: Arc<dyn TeamRepository>,
    pub task_events: Arc<dyn TaskEventRepository>,
    pub task_templates: Arc<dyn TaskTemplateRepository>,
    pub plugin_repos: Arc<dyn PluginRepoRepository>,
    /// Skill discovery service — reads `.claude/skills/` from workspace
    /// and plugin repositories, parses frontmatter, caches results.
    pub skills: Arc<SkillService>,
}

impl AppState {
    /// Build `AppState` with Postgres-backed repositories.
    ///
    /// Wires all `Arc<dyn …Repository>` from a single connection pool,
    /// keeping the concrete `Pg*` types private to this constructor.
    pub fn new(
        pool: DbPool,
        config: Arc<Config>,
        sandbox: Arc<dyn SandboxProvider>,
        events: Option<Arc<dyn EventPublisher>>,
        s3: Option<aws_sdk_s3::Client>,
        adapters: Option<Arc<Adapters>>,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Self {
        let github_app_service = github_apps.default_service();

        let repositories: Arc<dyn RepoRepository> = Arc::new(PgRepoRepository(pool.clone()));
        let plugin_repos: Arc<dyn PluginRepoRepository> =
            Arc::new(PgPluginRepoRepository(pool.clone()));
        let provider_tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(PgProviderTokenRepository(pool.clone()));
        let provider_token_auth: Arc<dyn ProviderTokenAuthRepository> =
            Arc::new(PgProviderTokenAuthRepository(pool.clone()));
        let secret_records: Arc<dyn SecretRepository> = Arc::new(PgSecretRepository(pool.clone()));
        let secret_audits: Arc<dyn SecretAuditRepository> =
            Arc::new(PgSecretAuditRepository(pool.clone()));
        let secret_cipher = LocalEnvelopeSecretCipher::from_config(&config.secrets);
        let secrets = Arc::new(SecretService::new(
            secret_records.clone(),
            secret_audits.clone(),
            secret_cipher,
        ));

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
            CredentialService::new(
                provider_tokens.clone(),
                Some(provider_token_auth.clone()),
                github_app_service.clone(),
                Some(BroadcastContext {
                    provider: config.sandbox_provider,
                    sandbox_namespace: config.sandbox.namespace.clone(),
                    sandbox_grpc_port: config.sandbox.grpc_port,
                    tasks: Arc::new(PgTaskRepository(pool.clone())),
                }),
            )
            .with_codex_refresh(pool.clone(), codex_http),
        );

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
        let skills = Arc::new(SkillService::new(
            repositories.clone(),
            plugin_repos.clone(),
            provider_tokens.clone(),
            provider_token_auth.clone(),
            github_app_service.clone(),
            skill_fetcher,
            std::time::Duration::from_secs(config.skill.cache_ttl_secs),
        ));

        Self {
            tasks: Arc::new(PgTaskRepository(pool.clone())),
            task_relations: Arc::new(PgTaskRelationRepository(pool.clone())),
            messages: Arc::new(PgMessageRepository(pool.clone())),
            feedback: Arc::new(PgFeedbackRepository(pool.clone())),
            repositories,
            commands: Arc::new(PgCommandRepository(pool.clone())),
            tags: Arc::new(PgTagRepository(pool.clone())),
            files: Arc::new(PgFileRepository(pool.clone())),
            figma_imports: Arc::new(PgFigmaRepository(pool.clone())),
            users: Arc::new(PgUserRepository(pool.clone())),
            provider_tokens,
            provider_token_auth,
            secret_records,
            secret_audits,
            secrets,
            teams: Arc::new(PgTeamRepository(pool.clone())),
            task_events: Arc::new(PgTaskEventRepository(pool.clone())),
            task_templates: Arc::new(PgTaskTemplateRepository(pool.clone())),
            plugin_repos,
            skills,
            db: pool,
            config,
            sandbox,
            events,
            s3,
            adapters,
            github_app_service,
            github_apps,
            credentials_svc,
            recently_sent: Arc::new(RecentlySent::new()),
            linear_active_turns: Arc::new(ActiveLinearTurns::new()),
            tool_call_names: Arc::new(
                moka::future::Cache::builder()
                    .max_capacity(50_000)
                    .time_to_live(std::time::Duration::from_secs(1800))
                    .build(),
            ),
        }
    }
}
