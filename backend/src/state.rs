//! Unified application state for API routes.
//!
//! All handler-facing database access goes through repository trait objects,
//! enabling full unit-test mocking without a real database.

use std::sync::Arc;

use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::users::UserRepository;
use crate::channel::config_dao::ChannelConfigRepository;
use crate::channel::provider::ChannelProviders;
use crate::channel::thread_dao::ChannelThreadRepository;
use crate::command::dao::CommandRepository;
use crate::config::Config;
use crate::db::DbPool;
use crate::events::EventPublisher;
use crate::figma::dao::FigmaRepository;
use crate::file_storage::dao::FileRepository;
use crate::message::dao::MessageRepository;
use crate::repository::dao::RepoRepository;
use crate::sandbox::provider::SandboxProvider;
use crate::tag::dao::TagRepository;
use crate::task::dao::TaskRepository;

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
