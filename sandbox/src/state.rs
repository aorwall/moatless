//! Shared application state for the sandbox service.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::agent::agent_state::AgentState;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::events::GrpcEventPublisher;
use crate::init::InitPhase;
use crate::server::registry::ServerRegistry;

/// Shared state passed to all axum handlers via `State` extractor.
///
/// All fields are cheap to clone (Arc-wrapped or internally Arc'd).
#[derive(Clone)]
pub struct SandboxState {
    /// Immutable configuration.
    pub config: Arc<Config>,
    /// gRPC event publisher (replaces Redis event publisher).
    pub events: GrpcEventPublisher,
    /// Agent execution state (status, session, cancel token, pending prompts).
    pub agent: Arc<AgentState>,
    /// Notification channel to wake the message polling consumer.
    pub consumer_notify: Arc<tokio::sync::Notify>,
    /// Server process registry (file-explorer, preview, etc.).
    pub servers: Arc<ServerRegistry>,
    /// gRPC client for backend communication (message polling).
    pub backend_client: BackendClient,
    /// Current initialization phase — tracked for health probes and consumer gating.
    pub init_phase: Arc<RwLock<InitPhase>>,
    /// Credentials fetched during init, passed to agent subprocess via `ClaudeOptions.env`.
    pub credentials: Arc<RwLock<HashMap<String, String>>>,
}
