//! Unified application state for API routes.
//!
//! All handler-facing database access goes through repository trait objects,
//! enabling full unit-test mocking without a real database.

use std::sync::Arc;

use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use crate::auth::users::{PgUserRepository, UserRepository};
use crate::channel::config_dao::{ChannelConfigRepository, PgChannelConfigRepository};
use crate::channel::provider::ChannelProviders;
use crate::channel::thread_dao::{ChannelThreadRepository, PgChannelThreadRepository};
use crate::command::dao::{CommandRepository, PgCommandRepository};
use crate::config::Config;
use crate::db::DbPool;
use crate::events::EventPublisher;
use crate::figma::dao::{FigmaRepository, PgFigmaRepository};
use crate::file_storage::dao::{FileRepository, PgFileRepository};
use crate::message::dao::{MessageRepository, PgMessageRepository};
use crate::repository::dao::{PgRepoRepository, RepoRepository};
use crate::sandbox::provider::SandboxProvider;
use crate::tag::dao::{PgTagRepository, TagRepository};
use crate::task::dao::{PgTaskRepository, TaskRepository};

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
    /// Optional channel providers for Slack/Email integrations.
    pub channel_providers: Option<Arc<ChannelProviders>>,

    // =========================================================================
    // Repository trait objects
    // =========================================================================
    pub tasks: Arc<dyn TaskRepository>,
    pub messages: Arc<dyn MessageRepository>,
    pub repositories: Arc<dyn RepoRepository>,
    pub commands: Arc<dyn CommandRepository>,
    pub tags: Arc<dyn TagRepository>,
    pub files: Arc<dyn FileRepository>,
    pub figma_imports: Arc<dyn FigmaRepository>,
    pub channel_configs: Arc<dyn ChannelConfigRepository>,
    pub channel_threads: Arc<dyn ChannelThreadRepository>,
    pub users: Arc<dyn UserRepository>,
    pub provider_tokens: Arc<dyn ProviderTokenRepository>,
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
        channel_providers: Option<Arc<ChannelProviders>>,
    ) -> Self {
        Self {
            tasks: Arc::new(PgTaskRepository(pool.clone())),
            messages: Arc::new(PgMessageRepository(pool.clone())),
            repositories: Arc::new(PgRepoRepository(pool.clone())),
            commands: Arc::new(PgCommandRepository(pool.clone())),
            tags: Arc::new(PgTagRepository(pool.clone())),
            files: Arc::new(PgFileRepository(pool.clone())),
            figma_imports: Arc::new(PgFigmaRepository(pool.clone())),
            channel_configs: Arc::new(PgChannelConfigRepository(pool.clone())),
            channel_threads: Arc::new(PgChannelThreadRepository(pool.clone())),
            users: Arc::new(PgUserRepository(pool.clone())),
            provider_tokens: Arc::new(PgProviderTokenRepository(pool.clone())),
            db: pool,
            config,
            sandbox,
            events,
            s3,
            channel_providers,
        }
    }
}
