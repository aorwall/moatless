//! Unified application state for the new API routes.

use std::sync::Arc;

use crate::channel::provider::ChannelProviders;
use crate::config::Config;
use crate::db::DbPool;
use crate::events::RedisEventPublisher;
use crate::sandbox::provider::SandboxProvider;

/// Shared application state for task/message/agent/command/file routes.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd).
#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL connection pool (Arc internally).
    pub db: DbPool,
    /// Immutable configuration shared across handlers.
    pub config: Arc<Config>,
    /// Sandbox workspace provider (trait object for K8s or future backends).
    pub sandbox: Arc<dyn SandboxProvider>,
    /// Optional Redis event publisher for real-time event streaming.
    pub events: Option<RedisEventPublisher>,
    /// Optional S3 client for file uploads/downloads.
    pub s3: Option<aws_sdk_s3::Client>,
    /// Optional channel providers for Slack/Email integrations.
    pub channel_providers: Option<Arc<ChannelProviders>>,
}
