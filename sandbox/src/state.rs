//! Shared application state for the sandbox service.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentHandle;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::events::GrpcEventPublisher;
use crate::init::InitPhase;

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
    /// Actor-style agent handle — send messages via `agent_handle.send()`.
    pub agent_handle: AgentHandle,
    /// Backend client for gRPC communication (message polling, acknowledgment).
    pub backend_client: Arc<dyn BackendClient>,
    /// Current initialization phase — tracked for health probes and consumer gating.
    pub init_phase: Arc<RwLock<InitPhase>>,
    /// Credentials fetched during init, passed to agent subprocess via `ClaudeOptions.env`.
    pub credentials: Arc<RwLock<HashMap<String, String>>>,
}
