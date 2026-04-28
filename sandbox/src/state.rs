//! Shared application state for the sandbox service.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentHandle;
use crate::agent::scheduled_job::ScheduledJobRegistry;
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
    /// Registry of deferred-work jobs (Monitor crons + ScheduleWakeup timers)
    /// spawned by the permission handler / agent actor.
    pub scheduled_jobs: Arc<ScheduledJobRegistry>,
    /// Backend client for gRPC communication (message polling, acknowledgment).
    pub backend_client: Arc<dyn BackendClient>,
    /// Current initialization phase — tracked for health probes and consumer gating.
    pub init_phase: Arc<RwLock<InitPhase>>,
    /// Credentials fetched during init, passed to agent subprocess via `ClaudeOptions.env`.
    ///
    /// Git-provider tokens (`GITHUB_TOKEN`, `GH_TOKEN`, `GIT_TOKEN`) are intentionally
    /// stripped from this map before it reaches the agent subprocess — git/gh operations
    /// inside the agent read tokens fresh from disk (`~/.config/gh/hosts.yml`) so inherited
    /// env would only risk staleness across long-running turns.
    pub credentials: Arc<RwLock<HashMap<String, String>>>,
    /// Cancellation token tied to the sandbox lifecycle.
    ///
    /// Used by background tasks (e.g. the credential refresher) to shut down
    /// cleanly when the pod is stopping.
    pub lifecycle_cancel: CancellationToken,
}
