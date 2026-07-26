//! Shared application state for the sandbox service.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentHandle;
use crate::agent::heartbeat::Heartbeat;
use crate::agent::scheduled_job::ScheduledJobRegistry;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;
use crate::init::codex_watcher::WatcherHandle;
use crate::init::InitPhase;
use crate::supervisor::SupervisorHealth;

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
    /// Liveness heartbeat bumped by the agent actor loop. Read by the `/livez`
    /// probe to detect a wedged actor (see [`crate::agent::heartbeat`]).
    pub heartbeat: Heartbeat,
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
    /// Generic dispatcher for backend-pushed credential refreshes.
    ///
    /// The gRPC server hands the rotated value to this dispatcher, which
    /// routes to the appropriate sink (file write, env var update, etc.).
    pub credential_dispatcher: Arc<Dispatcher>,
    /// Tracks credential failures observed on agent stderr (e.g. Codex
    /// `refresh_token_reused`). The agent loop reads + clears these flags at
    /// turn-failure boundaries to decide whether to retry after pulling a
    /// fresh credential from the backend.
    pub credential_failures: CredentialFailureDetector,
    /// Handle to the Codex auth watcher, used by [`crate::credentials::sinks`]
    /// to seed the watcher cache after a backend-driven write so the resulting
    /// filesystem event is recognised as a no-op.
    pub codex_watcher_handle: Arc<RwLock<Option<WatcherHandle>>>,
    /// Last system prompt fetched from the backend. Used as a fallback when
    /// `BackendClient::get_system_prompt` fails on a subsequent agent run.
    /// Refreshed on every successful fetch in `start_session`.
    pub system_prompt_cache: Arc<RwLock<Option<String>>>,
    /// Single-flight guard for pending-message dispatch.
    ///
    /// `get_pending_messages` only returns still-unacknowledged rows, and the ack
    /// lands asynchronously after the agent accepts the input. Without this, two
    /// dispatchers racing at the Ready transition (the authoritative startup
    /// dispatch and a gRPC notification) each fetch the same unacked Message and
    /// submit it twice. A dispatcher holds this guard from before the fetch until
    /// its acknowledge completes, so a concurrent one blocks and then fetches
    /// empty — the Message reaches the harness exactly once.
    pub dispatch_lock: Arc<Mutex<()>>,
    /// Liveness view of the root supervisor's tasks, read by `/livez`.
    ///
    /// Non-empty means a task declared [`Criticality::Fatal`] or
    /// [`Criticality::Liveness`] terminated before shutdown; see
    /// [`SandboxState::is_live`].
    ///
    /// [`Criticality::Fatal`]: crate::supervisor::Criticality::Fatal
    /// [`Criticality::Liveness`]: crate::supervisor::Criticality::Liveness
    pub supervisor_health: Arc<SupervisorHealth>,
}

impl SandboxState {
    /// The `/livez` decision: is this pod still able to make progress?
    ///
    /// Two independent ways to lose that, both of which want the same remedy —
    /// replace the pod:
    ///
    /// 1. A supervised task the pod needs terminated early. The supervisor
    ///    records it by name; nothing here is restartable in place.
    /// 2. The agent actor wedged or exited — see
    ///    [`crate::agent::heartbeat::actor_is_alive`].
    ///
    /// Deliberately still independent of backend reachability, credentials, and
    /// disk: a supervised task's termination is *this process* being broken, not
    /// a dependency being briefly unavailable, so a transient outage still
    /// cannot trigger a restart that kills an in-flight turn.
    pub async fn is_live(&self) -> bool {
        use crate::agent::heartbeat::{actor_is_alive, now_millis, HEARTBEAT_STALE_THRESHOLD_MS};
        use crate::agent::types::AgentStatus;

        let failed = self.supervisor_health.failed_tasks();
        if !failed.is_empty() {
            warn!(?failed, "Liveness: supervised tasks terminated early");
            return false;
        }

        let turn_running = matches!(self.agent.status().await, AgentStatus::Running);
        actor_is_alive(
            now_millis(),
            self.heartbeat.last_beat_ms(),
            turn_running,
            !self.agent_handle.is_closed(),
            HEARTBEAT_STALE_THRESHOLD_MS,
        )
    }

    /// Whether core init has finished (Session state restored, workspace cloned)
    /// and the pod has reached [`InitPhase::Ready`].
    ///
    /// The message dispatcher gates on this so a backend notification that races
    /// init (the gRPC server accepts calls before `run_init` completes) can never
    /// drive the harness before restoration — `run_init` performs the
    /// authoritative pending dispatch itself once Ready.
    pub(crate) async fn is_ready(&self) -> bool {
        matches!(*self.init_phase.read().await, InitPhase::Ready)
    }
}
