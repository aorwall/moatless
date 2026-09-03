#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
//! Opt-in T3 Code UI compatibility routes.
//!
//! The anti-corruption layer between T3's contract and Moatless state: the only
//! place in the backend that knows T3's vocabulary, which appears on the wire
//! and nowhere behind it.
//!
//! The shell listing projects the conversations a viewer follows and the
//! workspaces they run against, and reports the position it read them at. A
//! reconnecting client passes that position as `afterSequence` and is replayed
//! from it while the changes it missed are retained; past that it is handed a
//! fresh listing, a different member of the stream union rather than an empty
//! answer.
//!
//! Opening a conversation projects its transcript into the two collections T3
//! renders — what was said as messages, what was done as a work log — over an
//! HTTP snapshot plus a subscription on its own `task_activities` change feed.
//! Positions are per feed: a cursor from one is refused by the other. See
//! [`ThreadFeed`].

pub mod activity;
pub mod activity_dao;
pub mod agent;
mod assets;
mod attachments;
pub mod auth;
mod commands;
pub mod dao;
pub mod task_dao;
pub mod workspace_dao;
mod feed;
pub mod files;
mod git_actions;
pub mod listing;
mod oauth;
pub mod ports;
mod preview;
pub mod preview_tabs;
mod projection;
mod providers;
mod review;
mod sandbox;
mod scripts;
mod servers;
mod subtasks;
mod terminal;
pub mod workspace;

pub use feed::{ShellFeed, ThreadFeed};
use moatless_backend_core::sandbox::SandboxError;
pub use moatless_backend_core::sandbox::{ClusterDiagnostics, SandboxIntentReader, SandboxObserver, SandboxPortPublisher, SandboxStarter, SandboxStopper};
pub use sandbox_proto::client::SandboxConnector;
pub use moatless_backend_core::ui_rpc::{UiRpcAuthIdentity, UiRpcAuthMethod, UserRole};
pub use moatless_identity::auth::pairing::DevicePairing;

pub type UiRpcDbPool = sqlx::PgPool;

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tower_http::compression::CompressionLayer;
use tracing::{Instrument, debug, error, info, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::agent::catalog::UiRpcAgentKind;
use crate::dao::{Conversation, ConversationDao, Transcript};
use crate::task_dao::T3TaskDao;
use crate::workspace_dao::T3WorkspaceDao;
use crate::listing::{UiRpcEnvironmentIdentity, UiRpcResume};
use crate::preview::{PreviewError, PreviewState};
use crate::preview_tabs::PreviewTabDao;
use crate::sandbox::{SandboxRpcError, SandboxState};
use crate::scripts::{ScriptsError, ScriptsState};
use crate::servers::{ServersError, ServersState};
use crate::subtasks::{SubtasksError, SubtasksState};
use crate::workspace::WorkspaceAccessError;
use moatless_backend_core::change_feed::conversation::{ConversationHead, Resume, resume};
use moatless_types::access_control::ScopeContext;

/// The largest WebSocket message this socket reads or writes.
///
/// Sized by the one frame that is not small: T3 has no upload endpoint, so a
/// composer attachment travels inside `orchestration.dispatchCommand` as a
/// base64 data URL of up to 14,000,000 characters. Oversized attachments are
/// refused by name in [`commands`]; a frame cap is only reached by turns the
/// contract allows, and it fails as a dropped socket rather than a refusal.
/// Deliberately below eight maximum-size images: the composer re-encodes before
/// sending, and this buffer is allocated for anyone who can open the socket.
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_MESSAGES_PER_FRAME: usize = 128;
const MAX_ACTIVE_STREAMS: usize = 64;

/// How often a shell subscription asks what has changed.
///
/// This interval is also the coalescing window: everything recorded for one
/// conversation inside it collapses into a single re-read of that row. Short
/// enough that a turn starting looks immediate, long enough that an agent
/// working flat out cannot turn the sidebar into a delivery loop.
const SHELL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How often an open conversation asks whether it has moved.
///
/// Longer than the shell's because what it delivers is larger: the shell sends
/// one changed row, a conversation sends itself whole (see [`ThreadFeed`]). The
/// question asked each interval is cheap either way — a fingerprint, not a
/// transcript — so this is the coalescing window for the *answer*, and a second
/// is the shortest one at which an agent writing continuously does not turn a
/// transcript into a delivery loop.
const THREAD_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How often an open conversation asks its Sandbox about its branch state.
///
/// Each pass is a gRPC round trip to a pod that runs `git` for it, so this is
/// far longer than the two above: nothing on the screen is refreshed by it that
/// the client is not already refetching on its own, and the panel that reads it
/// most only needs it once, to pick which scope it opens on.
const VCS_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// Frames a subscription may have queued for the socket before it is dropped.
///
/// A shell subscription produces at most one chunk per poll, so this is deep
/// enough to absorb several seconds of a stalled writer; past that the poller
/// waits, which is the correct backpressure — the next poll coalesces whatever
/// accumulated while it was blocked into the same single row.
const OUTBOUND_FRAMES: usize = 64;

/// The T3 Code client release this module's surface matches. A client refuses
/// a server reporting a different version.
///
/// Move it in the change that brings the surface up to a newer client.
const T3_CLIENT_CONTRACT_VERSION: &str = "0.0.31";

/// The readable half of the environment id, and the name the client displays.
/// The digest [`environment_id`] appends is what tells environments apart.
const T3_ENVIRONMENT_NAME: &str = "moatless";
const T3_ENVIRONMENT_LABEL: &str = "Moatless";

/// How many entries a workspace search answers with when the client asks for no
/// particular number.
///
/// The picker renders a scrolling list and the person picks from the top of it;
/// a hundred is past what anyone reads and well inside what one gRPC round trip
/// returns.
const SEARCH_ENTRIES_LIMIT: usize = 100;

/// The most entries one search may ask for, whatever the client requests.
///
/// The limit reaches the Sandbox, which walks the tree to satisfy it, so an
/// unbounded request is a way to make a pod work arbitrarily hard from a
/// keystroke.
const SEARCH_ENTRIES_MAX: usize = 200;

/// How many conversations one search answers with.
///
/// One row per Task, so this is fifty conversations rather than fifty messages
/// — the palette shows a handful and this leaves room to rank.
const SEARCH_THREADS_LIMIT: i64 = 50;

/// The shortest query worth running against every Message body.
///
/// A one-character search matches most of the corpus and tells the person
/// nothing, and it costs a full scan to say so. Below this the answer is empty
/// without a query.
const SEARCH_THREADS_MIN_QUERY: usize = 2;

#[derive(Debug, Clone)]
struct UiRpcConfig {
    environment_id: String,
    allowed_origins: Vec<AllowedOrigin>,
    /// Whether this deployment exports spans over OTLP — reported to the
    /// client as `observability.otlpTracesEnabled` (see [`server_config`]).
    otlp_traces_enabled: bool,
    /// Whether attachment uploads can be offered, which is whether a URL can be
    /// signed for one — see [`attachments`]. Reported as
    /// `capabilities.attachmentUploads`, and reported honestly: the composer
    /// stops sending images inline the moment it reads that key, so a
    /// deployment that could not then take the upload would lose the person
    /// their attachment rather than fall back.
    attachment_uploads: bool,
}

/// One entry of `T3_UI_RPC_ALLOWED_ORIGINS`.
///
/// An entry whose host is `*.<domain>` admits any host under that domain on the
/// same scheme. Sandbox previews are reached at a per-workspace hostname that
/// only exists once the workspace does, so naming each one is not something a
/// deployment can do in advance.
///
/// The wildcard is a host suffix and nothing more: it never spans a scheme, and
/// a port makes the origin a different one it does not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AllowedOrigin {
    Exact(String),
    /// `prefix` is `<scheme>://`, `suffix` is `.<domain>`.
    Subdomain {
        prefix: String,
        suffix: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum UiRpcConfigError {
    #[error("Invalid T3 UI RPC configuration: {0}")]
    Invalid(String),
    /// The environment's identity is read from the database, so a database that
    /// cannot be read has no identity and this module cannot be mounted. It
    /// fails startup rather than falling back to a configured constant: a
    /// deployment that quietly reverted to one would present a replaced
    /// database as the same environment, which is the failure the identity
    /// exists to make visible.
    #[error("The T3 UI RPC environment identity could not be read: {0}")]
    Unidentifiable(#[from] sqlx::Error),
}

/// The models this deployment forwards to Sandboxes when a conversation names
/// none, by kind of agent.
///
/// Step three of the chain a conversation's reported model is resolved through.
/// Both are optional configuration, so both may be absent.
#[derive(Debug, Clone, Default)]
pub struct AgentModelDefaults {
    pub claude_code: Option<String>,
    pub codex: Option<String>,
}

impl AgentModelDefaults {
    /// The deployment default for one kind of agent, if it configures one.
    pub(crate) fn for_kind(&self, kind: UiRpcAgentKind) -> Option<&str> {
        match kind {
            UiRpcAgentKind::ClaudeCode => self.claude_code.as_deref(),
            UiRpcAgentKind::Codex => self.codex.as_deref(),
        }
    }
}

#[derive(Clone)]
struct UiRpcState {
    rpc: Arc<UiRpcConfig>,
    /// The database this process started against, and therefore the database
    /// every position it has handed out was allocated in. Compared against what
    /// the database says it is on each resume; see [`task_listing::resume`].
    environment: Arc<UiRpcEnvironmentIdentity>,
    session_cookie_name: Arc<str>,
    /// This deployment's own web address, for the one link this module builds —
    /// a Task origin's link back into the Moatless UI.
    frontend_url: Arc<str>,
    /// Signs WebSocket tickets. Absent when the deployment configures no JWT
    /// secret, which is the same condition under which session cookies do not
    /// work either; the ticket route then answers that it cannot issue one.
    session_jwt_secret: Option<Arc<str>>,
    /// Turns a pairing code into the token a client on another device keeps.
    /// The issuing half lives with the rest of the account routes.
    pairing: Arc<DevicePairing>,
    workspaces: Arc<dyn T3WorkspaceDao>,
    conversations: Arc<dyn ConversationDao>,
    /// How a conversation's files are reached. The only piece of Moatless
    /// behind this module that is not a database read: a checkout exists inside
    /// its Sandbox and nowhere else.
    sandboxes: Arc<dyn SandboxConnector>,
    /// What a Task's servers are doing. Distinct from `sandboxes`, which
    /// reaches *into* a Sandbox; this only observes one from outside.
    servers: ServersState,
    /// Running a Workspace's declared Scripts: reads the scripts and publishes a
    /// script's port. The terminal it runs in is reached through `conversations`,
    /// `sandboxes` and `terminal`, the same handles `terminal.open` uses.
    scripts: ScriptsState,
    /// What a Task's Sandbox lifecycle is doing, and lifecycle commands.
    sandbox: SandboxState,
    /// The Tasks a Task spawned. A second fleet beside its subagents, and not
    /// the same kind of thing: each one is a Thread of its own.
    subtasks: SubtasksState,
    /// The browser tabs open on a Task, and the fan-out that tells every
    /// replica when one changes.
    preview: PreviewState,
    model_defaults: Arc<AgentModelDefaults>,
    /// The agent catalog, as this deployment can actually run it. Read on every
    /// config request because credentials come and go without a restart.
    agents: Arc<dyn ports::AgentCatalogSource>,
    /// A repository's branches, read from its git host, for a draft thread's
    /// branch picker before any Sandbox exists.
    branches: Arc<dyn ports::RepositoryBranchSource>,
    /// The pull request a Task is bound to, and reference resolution for the
    /// attach-PR dialog.
    pull_requests: Arc<dyn ports::BoundPullRequestSource>,
    /// The viewer's usable GitHub credential, for the Source Control panel's
    /// auth badge. Read per request because credentials come and go without a
    /// restart.
    source_control_auth: Arc<dyn ports::SourceControlAuthSource>,
    /// Carries out the commands the T3 client dispatches. Not this module's only
    /// write: a git action reaches a Sandbox through `sandboxes`.
    commands: Arc<dyn ports::ThreadCommandSink>,
    /// Where a chat attachment's bytes go, for the upload route and for
    /// `attachments.delete`.
    attachments: Arc<dyn ports::AttachmentStore>,
    /// In-process fan-out for terminal metadata and events observed through
    /// this backend instance.
    terminal: terminal::TerminalHub,
}

/// What this deployment is, for the parts of a thread projection that are not
/// read out of the conversation: the agent settings a thread reports where it
/// states none of its own, and the address a link back into Moatless is built
/// from. Grouped so a thread projection takes one value rather than two more
/// positional arguments.
pub struct Deployment<'a> {
    pub model_defaults: &'a AgentModelDefaults,
    pub frontend_url: &'a str,
}

impl UiRpcState {
    fn deployment(&self) -> Deployment<'_> {
        Deployment {
            model_defaults: &self.model_defaults,
            frontend_url: &self.frontend_url,
        }
    }
}

/// Build the T3 compatibility router when its environment gate is enabled.
///
/// Configuration stays at this HTTP boundary rather than becoming part of
/// sandbox or pod configuration. The global Moatless auth middleware protects
/// every route except the public discovery descriptor.
///
/// The one thing here that is *not* configuration is the environment's
/// identity, which is read from the database. See `environment_id`.
pub async fn router_from_env(
    dependencies: UiRpcDependencies,
) -> Result<Option<Router>, UiRpcConfigError> {
    if !env_enabled("T3_UI_RPC_ENABLED")? {
        return Ok(None);
    }

    let UiRpcDependencies {
        session_cookie_name,
        session_jwt_secret,
        pairing,
        workspaces,
        conversations,
        sandboxes,
        observer,
        diagnostics,
        ports,
        tasks,
        intent,
        starter,
        stopper,
        preview_tabs,
        model_defaults,
        agents,
        branches,
        relations,
        pull_requests,
        source_control_auth,
        commands,
        attachments,
        otlp_traces_enabled,
        frontend_url,
    } = dependencies;
    let environment = conversations.listing_head().await?.identity;
    let config = UiRpcConfig {
        environment_id: environment_id(&environment),
        allowed_origins: env::var("T3_UI_RPC_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(AllowedOrigin::parse)
            .collect(),
        otlp_traces_enabled,
        attachment_uploads: session_jwt_secret.is_some(),
    };
    config.validate()?;
    info!(
        environment_id = %config.environment_id,
        "T3 UI RPC compatibility routes registered"
    );
    Ok(Some(router(UiRpcState {
        rpc: Arc::new(config),
        environment: Arc::new(environment),
        session_cookie_name: Arc::from(session_cookie_name),
        frontend_url: Arc::from(frontend_url),
        session_jwt_secret: session_jwt_secret.map(Arc::from),
        pairing,
        workspaces: workspaces.clone(),
        conversations: conversations.clone(),
        sandboxes,
        servers: ServersState {
            observer: observer.clone(),
            tasks: tasks.clone(),
            intent: intent.clone(),
        },
        scripts: ScriptsState { workspaces, ports },
        sandbox: SandboxState {
            observer,
            tasks: tasks.clone(),
            intent,
            starter,
            stopper,
            diagnostics,
        },
        subtasks: SubtasksState {
            relations,
            conversations,
        },
        preview: PreviewState {
            tabs: preview_tabs,
            tasks,
        },
        model_defaults: Arc::new(model_defaults),
        agents,
        branches,
        pull_requests,
        source_control_auth,
        commands,
        attachments,
        terminal: terminal::TerminalHub::default(),
    })))
}

/// Everything the T3 compatibility routes need from the rest of the backend.
///
/// A struct rather than an argument list because all six are opaque handles
/// whose order carries no meaning at the call site.
pub struct UiRpcDependencies {
    /// Reported in the auth descriptor so a browser knows which cookie to send.
    pub session_cookie_name: String,
    /// The secret WebSocket tickets are signed with — the same one session
    /// cookies use. The root backend reads it from its auth configuration
    /// before mounting this router.
    pub session_jwt_secret: Option<String>,
    pub pairing: Arc<DevicePairing>,
    pub workspaces: Arc<dyn T3WorkspaceDao>,
    pub conversations: Arc<dyn ConversationDao>,
    /// Reaches a conversation's Sandbox, which is where its checkout lives.
    pub sandboxes: Arc<dyn SandboxConnector>,
    /// Reports what a Task's servers are doing, without reaching inside.
    pub observer: Arc<dyn SandboxObserver>,
    /// Fleet-wide Sandbox status and live resource usage, deployment-scoped
    /// rather than per-Task.
    pub diagnostics: Arc<dyn ClusterDiagnostics>,
    /// Publishes a Script's port on a running Sandbox. The Kubernetes provider
    /// patches the Service and creates the route; the Docker provider returns
    /// `None` (no way out of the fixed-port agent container).
    pub ports: Arc<dyn SandboxPortPublisher>,
    /// Authorizes every `servers.*`, `sandbox.*`, and `preview.*` call: a
    /// scoped read that finds nothing is the same answer as a Task that does
    /// not exist.
    pub tasks: Arc<dyn T3TaskDao>,
    /// The desired state a status read is resolved against.
    pub intent: Arc<dyn SandboxIntentReader>,
    /// Starts a Task's Sandbox through the same lifecycle path as the REST API.
    pub starter: Arc<dyn SandboxStarter>,
    /// Stops a Task's Sandbox through the same lifecycle path as the REST API.
    pub stopper: Arc<dyn SandboxStopper>,
    pub preview_tabs: Arc<dyn PreviewTabDao>,
    pub model_defaults: AgentModelDefaults,
    /// Which agents this viewer can run, and with which models.
    pub agents: Arc<dyn ports::AgentCatalogSource>,
    /// A repository's branches, read from its git host, for a draft thread's
    /// branch picker.
    pub branches: Arc<dyn ports::RepositoryBranchSource>,
    /// The child edges behind `subtasks.list`. Access is applied afterwards,
    /// by re-reading each child through `conversations`.
    pub relations: Arc<dyn ports::TaskRelationSource>,
    /// The pull request a Task is bound to, and reference resolution for the
    /// attach-PR dialog.
    pub pull_requests: Arc<dyn ports::BoundPullRequestSource>,
    /// The viewer's usable GitHub credential, for the Source Control panel.
    pub source_control_auth: Arc<dyn ports::SourceControlAuthSource>,
    /// Carries out the commands the T3 client dispatches.
    pub commands: Arc<dyn ports::ThreadCommandSink>,
    /// Where a chat attachment's bytes go. Uploads are stored as ordinary
    /// Moatless uploads, under the id `attachments.createUploadUrl` minted.
    pub attachments: Arc<dyn ports::AttachmentStore>,
    /// Whether this deployment exports spans over OTLP (an OTLP endpoint is
    /// configured — see `config::OtelConfig`), reported to the client as
    /// `observability.otlpTracesEnabled`.
    pub otlp_traces_enabled: bool,
    /// This deployment's own web address (`config.frontend_url`), for the link a
    /// Task origin carries back into the Moatless UI.
    pub frontend_url: String,
}

/// The identity this deployment presents itself by: a configured name, and a
/// digest of the database behind it.
///
/// T3 keys everything it caches for an environment on this string, read once
/// per page load, so it moves at reload rather than at reconnect. Same-origin
/// clients build their connection from the descriptor, so a changed id starts
/// the shell empty; a registered connection stored the id at onboarding and
/// refuses to connect on a mismatch. The guards in [`task_listing::resume`] and
/// [`ShellFeed::poll`](feed::ShellFeed::poll) make a mid-session replacement
/// safe.
///
/// The name ([`T3_ENVIRONMENT_NAME`]) is the readable half; the digest tells one
/// environment from another. Two deployments sharing a database are one
/// environment.
fn environment_id(identity: &UiRpcEnvironmentIdentity) -> String {
    format!("{T3_ENVIRONMENT_NAME}-{}", identity.digest())
}

fn router(state: UiRpcState) -> Router {
    // The client pulls a thread body over HTTP rather than the socket so the
    // transport can compress it (T3's `threadSnapshotHttp.ts`), and unencoded a
    // long conversation is ~2MB against ~400KB gzipped. Scoped to the JSON reads
    // rather than applied globally because `/ws` shares this router.
    let compression = CompressionLayer::new();
    Router::new()
        .route("/.well-known/t3/environment", get(environment_descriptor))
        .route("/oauth/token", post(oauth::token_exchange))
        .route("/api/auth/websocket-ticket", post(oauth::websocket_ticket))
        .route("/api/auth/session", get(auth_session))
        .route(
            &format!(
                "{}/{{thread_id}}/{{*relative_path}}",
                assets::ASSET_ROUTE_PREFIX
            ),
            get(assets::serve),
        )
        // The token in the path is the credential — see [`attachments`] — so
        // this one route is exempt from the auth middleware's cookie handling
        // rather than authenticated by it.
        .route(
            &format!("{}/{{token}}", attachments::UPLOAD_ROUTE_PREFIX),
            post(attachments::receive),
        )
        .route(
            "/api/orchestration/shell",
            get(shell_snapshot_http).layer(compression.clone()),
        )
        .route(
            "/api/orchestration/threads/{thread_id}",
            get(thread_snapshot_http).layer(compression.clone()),
        )
        .route(
            "/api/orchestration/scripts/{thread_id}",
            get(scripts_list_http).layer(compression),
        )
        .route(
            "/api/orchestration/scripts/{thread_id}/{script_id}/run",
            post(scripts_run_http),
        )
        .route("/ws", get(websocket_upgrade))
        .with_state(state)
}

impl UiRpcConfig {
    fn validate(&self) -> Result<(), UiRpcConfigError> {
        if self.allowed_origins.is_empty() {
            return Err(UiRpcConfigError::Invalid(
                "T3_UI_RPC_ALLOWED_ORIGINS must contain at least one origin".into(),
            ));
        }
        Ok(())
    }
}

fn env_enabled(key: &str) -> Result<bool, UiRpcConfigError> {
    match env::var(key) {
        Err(env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(UiRpcConfigError::Invalid(format!(
            "{key} could not be read: {error}"
        ))),
        Ok(value) => parse_enabled(key, &value),
    }
}

fn parse_enabled(key: &str, value: &str) -> Result<bool, UiRpcConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(UiRpcConfigError::Invalid(format!(
            "{key} must be true or false"
        ))),
    }
}

async fn environment_descriptor(State(state): State<UiRpcState>) -> Json<Value> {
    Json(descriptor(&state.rpc))
}

async fn auth_session(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
) -> Json<Value> {
    debug!(user_id = %identity.user_id, "T3 UI session bootstrap");
    Json(session_state(&state, method.map(|Extension(m)| m)))
}

async fn shell_snapshot_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
) -> Response {
    debug!(user_id = %identity.user_id, "T3 UI shell snapshot bootstrap");
    match shell_snapshot(
        state.workspaces.as_ref(),
        state.conversations.as_ref(),
        &state.model_defaults,
        &viewer_scope(&identity),
    )
    .await
    {
        Ok(snapshot) => Json(snapshot.value).into_response(),
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "Failed to assemble T3 UI shell snapshot");
            (
                shell_failure_status(&error),
                Json(json!({ "error": "Failed to assemble the shell listing" })),
            )
                .into_response()
        }
    }
}

/// The transcript bootstrap, fetched before the socket subscription opens.
///
/// A 404 here is not an error the client reports: it defers to the socket
/// subscription and logs at debug (`state/threadSnapshotHttp.ts:99-107`). So
/// this status is also what a conversation the viewer may not see gets — the
/// alternative, a 403, would confirm the id exists.
async fn thread_snapshot_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path(thread_id): Path<String>,
) -> Response {
    debug!(user_id = %identity.user_id, task_id = %thread_id, "T3 UI thread snapshot bootstrap");
    match thread_snapshot(
        state.workspaces.as_ref(),
        state.conversations.as_ref(),
        &state.deployment(),
        &viewer_scope(&identity),
        &thread_id,
    )
    .await
    {
        Ok(Some(snapshot)) => Json(snapshot.value).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Conversation not found" })),
        )
            .into_response(),
        Err(error) => {
            error!(
                %error,
                user_id = %identity.user_id,
                task_id = %thread_id,
                "Failed to assemble T3 UI thread snapshot"
            );
            (
                shell_failure_status(&error),
                Json(json!({ "error": "Failed to assemble the conversation" })),
            )
                .into_response()
        }
    }
}

/// A Task's declared Scripts, for a caller outside the RPC/WS surface (`moat
/// tasks scripts list`) — the same read `scripts.run` resolves a Script id
/// against.
async fn scripts_list_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path(thread_id): Path<String>,
) -> Response {
    let viewer = viewer_scope(&identity);
    match scripts::list(&state.scripts, &viewer, &thread_id).await {
        Ok(scripts) => {
            Json(json!({ "scripts": scripts::public_script_list(&scripts) })).into_response()
        }
        Err(error) => scripts_error_response(&error),
    }
}

/// Start a Task's declared Script from outside the RPC/WS surface (`moat
/// tasks scripts run`). Same effect as `scripts.run`: execs into a named
/// terminal session, publishing the Script's port first if it declares one.
async fn scripts_run_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, script_id)): Path<(String, String)>,
) -> Response {
    let viewer = viewer_scope(&identity);
    let payload = json!({ "threadId": thread_id, "scriptId": script_id });
    match scripts::run(
        &state.scripts,
        state.conversations.as_ref(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => scripts_error_response(&error),
    }
}

/// Assemble the shell listing one viewer sees.
///
/// This is the module's data-access seam, and the whole of its internal
/// interface: read the conversations this viewer follows, read the workspaces
/// they need in order to be grouped, and project both onto the wire.
///
/// The two halves are filtered by deliberately different rules, and the
/// asymmetry is load-bearing rather than an oversight. Conversations are the
/// ones the viewer **follows**, may still read, and that are still **open** — a
/// sidebar is about involvement in unfinished work, so an administrator's is
/// what they follow like everyone else's, and a closed Task leaves it the way an
/// unfollowed one does. See [`dao::ConversationDao::list_followed`].
/// Workspaces are the union of every one the viewer may **read** *plus* every
/// one referenced by a listed conversation, whether or not they may read it:
/// people follow work that runs in someone else's private workspace, and the
/// client silently drops any conversation whose project is missing from the
/// listing, with no error and nothing logged.
pub async fn shell_snapshot(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
) -> Result<ShellSnapshot, sqlx::Error> {
    // Before the rows, never after. A position read afterwards would sit above
    // a change the rows already reflect, and the event carrying that change
    // would be discarded by the client as stale — one row frozen, silently.
    // Read first, the worst case is a change delivered twice, and delivery is a
    // whole row, so twice is the same as once.
    let head = conversations.listing_head().await?;
    let sequence = head.position;
    let conversations = conversations.list_followed(viewer).await?;
    let referenced_workspace_ids: Vec<String> = conversations
        .iter()
        .filter_map(|conversation| conversation.workspace_id.clone())
        .collect();
    let workspaces = workspaces
        .list_readable_including(viewer, &referenced_workspace_ids)
        .await?;
    Ok(ShellSnapshot {
        value: projection::shell_snapshot(&workspaces, &conversations, model_defaults, sequence),
        sequence,
        identity: head.identity,
    })
}

/// One conversation this viewer may read, together with the project a client
/// would file it under — or nothing.
///
/// The project is the second condition, not a detail of the first. A client
/// groups every conversation under a project it already holds, so one whose
/// workspace is not in the listing has nowhere to go and is dropped here rather
/// than sent to be silently discarded on arrival.
///
/// `Ok(None)` collapses every way a conversation can be absent for this viewer
/// into one answer; distinguishing them would disclose which.
async fn readable_under_a_project(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<(Conversation, String)>, sqlx::Error> {
    let Some(conversation) = conversations.readable_by_id(viewer, task_id).await? else {
        return Ok(None);
    };
    let referenced: Vec<String> = conversation.workspace_id.clone().into_iter().collect();
    let workspaces = workspaces
        .list_readable_including(viewer, &referenced)
        .await?;
    let project_id = conversation
        .workspace_id
        .as_deref()
        .filter(|id| workspaces.iter().any(|listed| listed.workspace.id == *id))
        .map(str::to_string);
    let Some(project_id) = project_id else {
        // The shell drops this conversation for the same reason, with the same
        // warning. Opening one the sidebar could not show it is the same gap
        // seen from the other side.
        warn!(
            task_id = %conversation.task_id,
            workspace_id = ?conversation.workspace_id,
            "Conversation has no listable workspace; not opening it in the T3 UI"
        );
        return Ok(None);
    };
    Ok(Some((conversation, project_id)))
}

/// One conversation's listing row, by id.
///
/// The listings answer "what am I involved in"; this answers "what is this
/// one", and those are different questions. A conversation reaches a client
/// through a link, a task tree or an archive long before it reaches the
/// sidebar — closed work never reaches the sidebar at all — and without this
/// the client holds a thread it can render a transcript for and cannot name.
///
/// Resolved by access rather than by Follow, exactly as [`thread_snapshot`] is:
/// the two are the same conversation seen at two levels of detail, so a
/// deployment where one opens and the other does not would be the bug.
pub async fn thread_shell(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<Value>, sqlx::Error> {
    Ok(
        readable_under_a_project(workspaces, conversations, viewer, task_id)
            .await?
            .map(|(conversation, project_id)| {
                projection::thread_shell(&conversation, &project_id, defaults)
            }),
    )
}

/// Assemble one conversation as the thread view carries it.
///
/// `Ok(None)` collapses every way a conversation can be absent for this viewer
/// into one answer; distinguishing them would disclose which.
///
/// The row is resolved by access ([`ConversationDao::readable_by_id`]), not by
/// Follow, so a link to a readable Task opens. The sidebar stays narrowed to
/// Follows until the reader writes. Reading is the whole of that widening —
/// running a turn resolves through [`commands::require_controllable`].
///
/// Read the change position **before** the rows, as [`shell_snapshot`] does:
/// read afterwards it sits above a change the rows already reflect and the
/// client discards that event as stale, freezing the conversation. Read first,
/// the worst case is one change delivered twice, and every event here is an
/// upsert.
pub async fn thread_snapshot(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<ThreadSnapshot>, sqlx::Error> {
    let head = conversations.conversation_head(task_id).await?;
    let Some((conversation, project_id)) =
        readable_under_a_project(workspaces, conversations, viewer, task_id).await?
    else {
        return Ok(None);
    };
    let project_id = project_id.as_str();
    let transcript = conversations.transcript(task_id).await?;
    if transcript.omitted > 0 {
        info!(
            task_id,
            omitted = transcript.omitted,
            "T3 UI thread snapshot left out the oldest messages of a long conversation"
        );
    }
    Ok(Some(ThreadSnapshot {
        value: json!({
            "snapshotSequence": head.sequence(),
            "thread": projection::thread_detail(
                &conversation,
                project_id,
                deployment.model_defaults,
                &transcript,
                &projection::OriginContext {
                    frontend_url: deployment.frontend_url,
                    source_task_names: &source_task_names(conversations, viewer, &transcript).await,
                },
            )
        }),
        head,
    }))
}

/// The names of the Tasks a window's Messages came from, by task id.
///
/// One scoped read, and only when a Message in the window came from another
/// Task at all — which nearly none do. A failure here costs the origins their
/// label and their link; it must not cost the reader the conversation, so it is
/// logged and answered with nothing.
async fn source_task_names(
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    window: &Transcript,
) -> std::collections::HashMap<String, String> {
    let mut ids: Vec<String> = window
        .messages
        .iter()
        .filter_map(|message| message.source_task_id.clone())
        .collect();
    if ids.is_empty() {
        return std::collections::HashMap::new();
    }
    ids.sort();
    ids.dedup();
    match conversations.source_task_names(viewer, &ids).await {
        Ok(names) => names,
        Err(error) => {
            warn!(%error, "T3 UI could not resolve the names of source tasks; origins go unlabelled");
            std::collections::HashMap::new()
        }
    }
}

/// One conversation as the client receives it, and the position it was built
/// at.
///
/// They travel together because a subscription needs both: the conversation
/// goes to the client and the position is where its feed starts watching.
pub struct ThreadSnapshot {
    pub value: Value,
    pub head: ConversationHead,
}

/// Open a subscription to one conversation: either replay what a returning
/// client missed, or hand it the conversation built afresh.
///
/// `after` is the sequence the client says it already holds — the
/// `snapshotSequence` of the bootstrap it fetched over HTTP moments earlier
/// (`state/threads.ts`), which is why the two have to be numbers in the same
/// namespace. [`resume`] is what decides between them, on the
/// same two grounds the shell's does: a sequence this conversation never
/// reached, and one further behind than the records kept.
///
/// The client separates the outcomes by decoding rather than by inference — a
/// rebuild is a `snapshot` item, which it applies by replacing what it holds,
/// and a replay is `event` items, which it folds in one at a time. So an
/// un-replayable gap is never silence.
///
/// # Why the catch-up is one ordinary poll
///
/// Replay reads through [`ThreadFeed::poll`], the same call the live
/// subscription makes every interval, so a resuming client is told exactly what
/// a client that never disconnected would have been told and there is no second
/// delivery path to drift from the first. The poll's own cap bounds the
/// catch-up, and the feed's cursor advances only over records it read, so
/// anything left over arrives on the next poll.
///
/// `Ok(None)` when the conversation is not visible to this viewer; the caller
/// answers the request rather than opening a stream that can only end.
pub async fn thread_subscription(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    deployment: &Deployment<'_>,
    viewer: &ScopeContext,
    task_id: &str,
    after: Option<i64>,
    request_completion_marker: bool,
) -> Result<Option<(Vec<Value>, ThreadFeed)>, sqlx::Error> {
    let head = conversations.conversation_head(task_id).await?;
    // Qualified: the shell has a decision of the same shape over a different
    // log, and the two cursors are not interchangeable.
    let resume = match after {
        Some(after) => resume(head, after),
        None => Resume::Rebuild,
    };
    if let Resume::From(cursor) = resume {
        // A replay reads nothing when nothing changed, and a poll that reads
        // nothing checks nothing. So the visibility rule is applied here as
        // well, or subscribing to a conversation this viewer may not see would
        // succeed for as long as it stayed still.
        if conversations.readable_by_id(viewer, task_id).await?.is_none() {
            return Ok(None);
        }
        let mut feed = ThreadFeed::resuming_from(task_id.to_string(), cursor);
        let Some(values) = feed
            .poll(conversations, viewer, deployment.frontend_url)
            .await?
        else {
            return Ok(None);
        };
        debug!(
            user_id = %viewer.user_id,
            task_id,
            cursor,
            replayed = values.len(),
            "T3 UI thread subscription resumed from the client's position"
        );
        return Ok(Some((
            thread_initial_values(values, request_completion_marker),
            feed,
        )));
    }

    let Some(snapshot) =
        thread_snapshot(workspaces, conversations, deployment, viewer, task_id).await?
    else {
        return Ok(None);
    };
    if let Some(after) = after {
        info!(
            user_id = %viewer.user_id,
            task_id,
            after,
            head = snapshot.head.sequence(),
            "T3 UI thread subscription could not replay from the client's position; \
             sending the conversation afresh"
        );
    }
    let feed = ThreadFeed::at(task_id.to_string(), snapshot.head);
    let values = vec![json!({
        "kind": "snapshot",
        "snapshot": snapshot.value
    })];
    Ok(Some((
        thread_initial_values(values, request_completion_marker),
        feed,
    )))
}

fn thread_initial_values(mut values: Vec<Value>, request_completion_marker: bool) -> Vec<Value> {
    if request_completion_marker {
        values.push(json!({ "kind": "synchronized" }));
    }
    values
}

/// A shell listing, the position it was read at, and the database it was read
/// from.
///
/// They travel together because a subscription needs all three: the listing goes
/// to the client, and the position and identity are where its live feed starts.
/// Splitting them is what lets the second be taken after the first, which is the
/// ordering mistake this whole mechanism exists to avoid.
pub struct ShellSnapshot {
    pub value: Value,
    pub sequence: i64,
    pub identity: UiRpcEnvironmentIdentity,
}

/// What a subscribing client is sent before live delivery starts, and the feed
/// that carries on from it.
pub struct ShellSubscription {
    /// The stream items for the first chunk, in the order the client applies
    /// them. Empty means the client was already up to date — and *only* that,
    /// which is the distinction this ticket exists to make.
    pub values: Vec<Value>,
    pub feed: ShellFeed,
}

/// Open a subscription: either replay what a returning client missed, or hand
/// it a listing built afresh.
///
/// `after` is the position the client says it already holds — T3's
/// `afterSequence`, absent for a client that holds nothing. `environment` is
/// the database this process started against; a database that no longer matches
/// it is [`listing::resume`]'s third reason to rebuild, and the only one
/// that catches a position handed out a second time to a different change.
///
/// # How the client tells a rebuild from silence
///
/// The two outcomes are different members of the contract's own
/// `OrchestrationShellStreamItem` union, so the client separates them by
/// decoding rather than by inference:
///
/// - **Replay** emits `thread-upserted` items, or none at all. The client folds
///   each into the snapshot it already holds (`shellReducer.ts`), keeping the
///   list it had. No items means nothing changed.
/// - **Rebuild** emits one `snapshot` item. The client *replaces* its snapshot
///   with it outright (`shell.ts:150-152`), which is what "freshly built
///   listing" means on that side.
///
/// So an un-replayable gap is never an empty answer — it is the same frame a
/// client with no snapshot at all receives (`apps/server/src/ws.ts:1297-1305`).
///
/// # The catch-up is one ordinary poll
///
/// Replay reads through [`ShellFeed::poll`], the same call the live
/// subscription makes every interval, so there is no second delivery path to
/// drift from the first. The poll's cap bounds the catch-up, and nothing can be
/// lost between the two: the feed's cursor advances only over records it read
/// and the same feed continues live, so anything left over arrives next poll.
pub async fn shell_subscription(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    environment: &UiRpcEnvironmentIdentity,
    after: Option<i64>,
) -> Result<ShellSubscription, sqlx::Error> {
    let head = conversations.listing_head().await?;
    let resume = match after {
        Some(after) => listing::resume(environment, &head.identity, head.position, after),
        None => UiRpcResume::Rebuild,
    };
    match resume {
        UiRpcResume::From(cursor) => {
            // Positioned, holding nothing it may take back, and grouping by the
            // projects this viewer can read. See `resume_grouping`.
            let grouping = resume_grouping(workspaces, viewer).await?;
            let mut feed = ShellFeed::resuming_from(head.identity, cursor).grouping_by(grouping);
            let values = feed
                .poll(workspaces, conversations, model_defaults, viewer)
                .await?;
            debug!(
                user_id = %viewer.user_id,
                cursor,
                replayed = values.len(),
                "T3 UI shell subscription resumed from the client's position"
            );
            Ok(ShellSubscription { values, feed })
        }
        UiRpcResume::Rebuild => {
            let snapshot =
                shell_snapshot(workspaces, conversations, model_defaults, viewer).await?;
            if let Some(after) = after {
                info!(
                    user_id = %viewer.user_id,
                    after,
                    head = snapshot.sequence,
                    retained = listing::RETAINED_CHANGES,
                    replaced_database = snapshot.identity != *environment,
                    "T3 UI shell subscription could not replay from the client's \
                     position; sending a freshly built listing"
                );
            }
            // What the client will be holding the moment it applies this, and
            // therefore the only ids its feed may ever take back. Read off the
            // projected listing rather than the rows behind it — see
            // [`ShellSnapshot::thread_ids`].
            let (threads, projects) = (snapshot.thread_ids(), snapshot.project_ids());
            let feed = ShellFeed::resuming_from(snapshot.identity, snapshot.sequence)
                .holding(threads, projects);
            Ok(ShellSubscription {
                feed,
                values: vec![json!({
                    "kind": "snapshot",
                    "snapshot": snapshot.value
                })],
            })
        }
    }
}

/// The projects a resuming client can be assumed to have a group for.
///
/// **Not the set of ids it may be sent a removal for**, which is a different
/// question with a different answer — see [`ShellFeed`]. This one only decides
/// whether a changed conversation can go out as a row or has to go out inside a
/// fresh listing, and getting it wrong costs a snapshot in one direction and a
/// row the client renders nowhere in the other.
///
/// A resuming client re-fetches the whole shell snapshot over HTTP and applies
/// it *before* it subscribes, sending that snapshot's own sequence as
/// `afterSequence` (`client-runtime/src/state/shell.ts:204-212`). So its
/// projects are the ones [`shell_snapshot`] returned, which is **every
/// workspace the viewer may read**, plus the ones a listed conversation
/// references.
///
/// This reads back the first half and not the second, and the asymmetry is the
/// point:
///
/// - A project the viewer may read now was in that snapshot unless it became
///   readable in between, and `ShellFeed::resynchronize` sends the whole
///   listing for a changed conversation entering it.
/// - A project held only because a conversation *references* it is left out.
///   That is the case where a conversation can arrive under a project the
///   client has never been sent, which `resynchronize` answers the same way.
///
/// Not covered: the viewer's own scope widening between their snapshot request
/// and their subscribe. The window is one round trip and the cost is one row
/// not rendered until the next rebuild, not a disclosure.
async fn resume_grouping(
    workspaces: &dyn T3WorkspaceDao,
    viewer: &ScopeContext,
) -> Result<Vec<String>, sqlx::Error> {
    Ok(workspaces
        .list_readable_including(viewer, &[])
        .await?
        .into_iter()
        .map(|listed| listed.workspace.id)
        .collect())
}

/// The workspace root a file request addresses.
///
/// The contract types `cwd` as a trimmed non-empty string, so a payload
/// carrying anything else did not come from a client encoding against it. It is
/// refused rather than defaulted: every value this module could substitute
/// names somebody's conversation.
fn workspace_cwd(payload: &Value) -> Option<String> {
    payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_owned)
}

/// The position a client says it already holds.
///
/// The contract types `afterSequence` as an optional `NonNegativeInt`, so a
/// value that is neither absent nor a non-negative integer cannot come from a
/// client that encodes against it. Reading one as *absent* is the safe
/// direction: the client is rebuilt, which is a listing it can use. Reading it
/// as a position would be resuming from a number nothing allocated.
fn after_sequence(payload: &Value) -> Option<i64> {
    payload
        .get("afterSequence")
        .and_then(Value::as_i64)
        .filter(|sequence| *sequence >= 0)
}

impl ShellSnapshot {
    /// The conversation ids this listing puts in front of the viewer.
    ///
    /// Read back out of the projected listing rather than off the rows it was
    /// built from, and deliberately: a conversation whose project is missing is
    /// dropped during projection, and a feed that believed the client held it
    /// would later "remove" a row that was never on the screen. What was sent is
    /// the only honest answer to what is held.
    pub fn thread_ids(&self) -> Vec<String> {
        self.ids("threads", "id")
    }

    /// The project ids this listing puts in front of the viewer.
    pub fn project_ids(&self) -> Vec<String> {
        self.ids("projects", "id")
    }

    fn ids(&self, collection: &str, key: &str) -> Vec<String> {
        self.value[collection]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row[key].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The archived half of the listing: the closed conversations the viewer
/// follows, under the same projects the live half carries.
///
/// Archiving is closing, so this is the live listing's complement — the same
/// follow and read rules at the other status. The projects come whole rather
/// than narrowed to the ones these conversations sit in, because the client
/// groups an archived row under a project it must already hold.
pub async fn archived_shell_snapshot(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
) -> Result<Value, sqlx::Error> {
    let sequence = conversations.listing_head().await?.position;
    let workspaces = workspaces.list_readable_including(viewer, &[]).await?;
    let archived = conversations.list_followed_closed(viewer).await?;
    Ok(projection::shell_snapshot(
        &workspaces,
        &archived,
        defaults,
        sequence,
    ))
}

/// The access scope the session's viewer reads Moatless rows with.
fn viewer_scope(identity: &UiRpcAuthIdentity) -> ScopeContext {
    ScopeContext {
        user_id: identity.user_id.to_string(),
        is_admin: matches!(identity.role, UserRole::Admin),
    }
}

async fn websocket_upgrade(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if origin_checked(method) && !origin_allowed(&headers, &state.rpc.allowed_origins) {
        warn!(
            user_id = %identity.user_id,
            origin = ?headers.get(header::ORIGIN),
            "Rejected T3 UI WebSocket origin"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "WebSocket origin is not allowed" })),
        )
            .into_response();
    }

    websocket
        .max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| serve_websocket(socket, state, identity))
}

/// Whether this upgrade's `Origin` has to be one we allow.
///
/// The allow-list is a defence against one thing: a page the person did not
/// write opening a socket that their cookie then authenticates. That is a
/// property of the cookie, not of the socket — a browser attaches it to any
/// request a page can cause, so the origin is the only thing left that says
/// which page caused it.
///
/// A ticket is the opposite. It has to be fetched first, with a bearer token
/// the page does not have and cannot read, so a request carrying one was made
/// by something holding the credential deliberately. Requiring an origin of it
/// would refuse every native client on principle: a React Native `WebSocket`
/// sends no `Origin` at all on Android, and an app has no origin to send.
///
/// An unknown method is treated as the cookie case. The only way to get here
/// without one is with the auth middleware unmounted, and that is not a
/// configuration whose sockets should be easier to open.
fn origin_checked(method: Option<Extension<UiRpcAuthMethod>>) -> bool {
    !matches!(method, Some(Extension(UiRpcAuthMethod::WebSocketTicket)))
}

fn origin_allowed(headers: &HeaderMap, allowed_origins: &[AllowedOrigin]) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            allowed_origins
                .iter()
                .any(|allowed| allowed.matches(origin))
        })
}

impl AllowedOrigin {
    fn parse(entry: &str) -> Self {
        match entry.split_once("://*.") {
            Some((scheme, domain)) if !scheme.is_empty() && !domain.is_empty() => Self::Subdomain {
                prefix: format!("{scheme}://"),
                suffix: format!(".{domain}"),
            },
            _ => Self::Exact(entry.to_string()),
        }
    }

    fn matches(&self, origin: &str) -> bool {
        match self {
            Self::Exact(allowed) => allowed == origin,
            Self::Subdomain { prefix, suffix } => origin
                .strip_prefix(prefix.as_str())
                // A browser sends a host and nothing else, but this header is
                // attacker-controlled: without these, `https://evil.example/
                // x.preview.moatless.xyz` ends with the suffix and passes.
                .is_some_and(|host| {
                    host.len() > suffix.len()
                        && host.ends_with(suffix.as_str())
                        && !host.contains(['/', '\\', '?', '#', '@', ':'])
                }),
        }
    }
}

/// Serve one connection: answer what the client asks, and send what a
/// subscription produces on its own.
///
/// The socket is split because those two are independent — a subscription's
/// chunk must not wait for the client to say something first, which is what the
/// request-response loop this replaces required.
async fn serve_websocket(socket: WebSocket, state: UiRpcState, identity: UiRpcAuthIdentity) {
    debug!(user_id = %identity.user_id, "T3 UI WebSocket connected");
    let (mut sink, mut incoming) = socket.split();
    let (frames, mut pushed) = mpsc::channel::<Value>(OUTBOUND_FRAMES);
    let mut session = ProtocolSession::new(frames);

    'connection: loop {
        let frames = tokio::select! {
            message = incoming.next() => {
                let Some(message) = message else { break };
                let output = match message {
                    Ok(Message::Text(text)) => {
                        session.handle_json(text.as_bytes(), &state, &identity).await
                    }
                    Ok(Message::Binary(bytes)) => {
                        session.handle_json(&bytes, &state, &identity).await
                    }
                    Ok(Message::Ping(payload)) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    Ok(Message::Pong(_)) => continue,
                    Ok(Message::Close(_)) => break,
                    Err(error) => {
                        debug!(%error, user_id = %identity.user_id, "T3 UI WebSocket receive failed");
                        break;
                    }
                };
                if output.eof {
                    send_frames(&mut sink, output.frames).await;
                    break;
                }
                output.frames
            }
            pushed = pushed.recv() => {
                // The sender is held by the session, which outlives this loop,
                // so `None` cannot arrive while the connection is open.
                match pushed {
                    Some(frame) => vec![frame],
                    None => break,
                }
            }
        };

        if !send_frames(&mut sink, frames).await {
            break 'connection;
        }
    }

    debug!(user_id = %identity.user_id, "T3 UI WebSocket disconnected");
}

/// Write frames to the socket, reporting whether the connection is still usable.
async fn send_frames(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    frames: Vec<Value>,
) -> bool {
    for frame in frames {
        let encoded = match serde_json::to_string(&frame) {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!(%error, "Failed to encode T3 UI RPC response");
                return false;
            }
        };
        if sink.send(Message::Text(encoded.into())).await.is_err() {
            return false;
        }
    }
    true
}

/// A request id, in the JSON form it arrived as and the text this protocol
/// correlates on.
///
/// T3 encodes the id as `string | number` and sends whichever it already had —
/// an unquoted integer for the counter its client generates. The form matters
/// on the way back: the client resolves a response by looking `requestId` up in
/// a map keyed by the value it sent, and its JSON codec coerces nothing. An id
/// that arrives as `0` and returns as `"0"` therefore matches no caller, the
/// request never completes, and the connection attempt times out with the
/// socket open and every frame apparently fine. So the id goes back exactly as
/// it came, while correlation here stays on the decimal text.
#[derive(Debug, Clone, PartialEq)]
struct RequestId {
    text: String,
    wire: Value,
}

impl RequestId {
    fn as_str(&self) -> &str {
        &self.text
    }

    fn wire(&self) -> Value {
        self.wire.clone()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RequestIdVisitor;

        impl serde::de::Visitor<'_> for RequestIdVisitor {
            type Value = RequestId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a request id as a string or an integer")
            }

            fn visit_str<E: serde::de::Error>(self, id: &str) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::String(id.to_string()),
                })
            }

            fn visit_u64<E: serde::de::Error>(self, id: u64) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::from(id),
                })
            }

            fn visit_i64<E: serde::de::Error>(self, id: i64) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::from(id),
                })
            }
        }

        // A fractional id reaches no visitor here and is refused, which is what
        // `valid_request_id` would decide about it anyway.
        deserializer.deserialize_any(RequestIdVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
enum ClientFrame {
    Request {
        id: RequestId,
        tag: String,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(rename = "traceId")]
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(rename = "spanId")]
        #[serde(default)]
        span_id: Option<String>,
        #[serde(default)]
        sampled: Option<bool>,
    },
    Ack {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Ping,
    Eof,
}

/// Root span for a single UI-RPC request. Honors a well-formed client trace
/// context (W3C-style 32-hex trace id / 16-hex span id) as the parent, so the
/// request's server-side spans land in the same trace as the client's. A
/// missing or malformed context mints a fresh root instead of failing the
/// request, and records the raw client-supplied value for debugging.
fn request_root_span(
    tag: &str,
    trace_id: Option<&str>,
    span_id: Option<&str>,
    sampled: Option<bool>,
) -> tracing::Span {
    let span = tracing::info_span!(
        "ui_rpc.request",
        tag = %tag,
        client_trace_id = tracing::field::Empty,
    );
    match remote_span_context(trace_id, span_id, sampled) {
        Some(cx) => {
            let _ = span.set_parent(cx);
        }
        None => {
            if let Some(raw) = trace_id {
                span.record("client_trace_id", raw);
            }
        }
    }
    span
}

/// Parses a client-supplied trace/span id pair into a remote OTel span
/// context, or `None` if either is missing or malformed. Never fails the
/// request — callers fall back to minting a fresh root span.
fn remote_span_context(
    trace_id: Option<&str>,
    span_id: Option<&str>,
    sampled: Option<bool>,
) -> Option<opentelemetry::Context> {
    use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState};

    // `from_hex` parses any valid hex string regardless of length (it does not
    // zero-pad), so a short client value like "deadbeef" would otherwise parse
    // into a spuriously "valid" trace id. The W3C format is fixed-width: reject
    // anything but exactly 32/16 hex characters before parsing.
    let trace_id = trace_id.filter(|s| s.len() == 32)?;
    let span_id = span_id.filter(|s| s.len() == 16)?;
    let trace_id = TraceId::from_hex(trace_id)
        .ok()
        .filter(|id| *id != TraceId::INVALID)?;
    let span_id = SpanId::from_hex(span_id)
        .ok()
        .filter(|id| *id != SpanId::INVALID)?;
    let flags = if sampled.unwrap_or(false) {
        TraceFlags::SAMPLED
    } else {
        TraceFlags::default()
    };
    let span_context = SpanContext::new(trace_id, span_id, flags, true, TraceState::default());
    Some(opentelemetry::Context::new().with_remote_span_context(span_context))
}

#[derive(Debug, Default)]
struct StreamState {
    awaiting_ack: bool,
    /// The task producing this stream's frames, for streams that produce any.
    /// Aborted when the client interrupts the request or the connection ends —
    /// a poller outliving its socket would query the database on behalf of
    /// nobody.
    producer: Option<JoinHandle<()>>,
}

impl Drop for StreamState {
    fn drop(&mut self) {
        if let Some(producer) = self.producer.take() {
            producer.abort();
        }
    }
}

#[derive(Debug)]
struct ProtocolSession {
    streams: HashMap<String, StreamState>,
    /// Where a stream's producer puts frames for the socket writer.
    frames: mpsc::Sender<Value>,
}

impl ProtocolSession {
    fn new(frames: mpsc::Sender<Value>) -> Self {
        Self {
            streams: HashMap::new(),
            frames,
        }
    }
}

#[derive(Debug, Default)]
struct ProtocolOutput {
    frames: Vec<Value>,
    eof: bool,
}

impl ProtocolSession {
    async fn handle_json(
        &mut self,
        bytes: &[u8],
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
    ) -> ProtocolOutput {
        let value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                return ProtocolOutput {
                    frames: vec![connection_defect(format!("Malformed JSON frame: {error}"))],
                    eof: false,
                };
            }
        };

        let values = match value {
            Value::Array(values) => values,
            value => vec![value],
        };
        if values.len() > MAX_MESSAGES_PER_FRAME {
            return ProtocolOutput {
                frames: vec![connection_defect(format!(
                    "A frame may contain at most {MAX_MESSAGES_PER_FRAME} messages"
                ))],
                eof: true,
            };
        }

        let mut output = ProtocolOutput::default();
        for value in values {
            let frame: ClientFrame = match serde_json::from_value(value) {
                Ok(frame) => frame,
                Err(error) => {
                    output.frames.push(connection_defect(format!(
                        "Malformed RPC envelope: {error}"
                    )));
                    continue;
                }
            };

            match frame {
                ClientFrame::Ping => output.frames.push(json!({ "_tag": "Pong" })),
                ClientFrame::Eof => {
                    output.eof = true;
                    break;
                }
                ClientFrame::Ack { request_id } => {
                    if !valid_request_id(request_id.as_str()) {
                        output
                            .frames
                            .push(connection_defect("Ack requestId is not a decimal string"));
                        continue;
                    }
                    if let Some(stream) = self.streams.get_mut(request_id.as_str()) {
                        stream.awaiting_ack = false;
                    }
                }
                ClientFrame::Interrupt { request_id } => {
                    if !valid_request_id(request_id.as_str()) {
                        output.frames.push(connection_defect(
                            "Interrupt requestId is not a decimal string",
                        ));
                        continue;
                    }
                    self.streams.remove(request_id.as_str());
                    output.frames.push(interrupted_exit(&request_id));
                }
                ClientFrame::Request {
                    id,
                    tag,
                    payload,
                    headers,
                    trace_id,
                    span_id,
                    sampled,
                } => {
                    // Not consumed: the sibling diagnostics spec, not this one.
                    drop(headers);
                    let span =
                        request_root_span(&tag, trace_id.as_deref(), span_id.as_deref(), sampled);
                    output.frames.extend(
                        self.handle_request(id, tag, payload, state, identity)
                            .instrument(span)
                            .await,
                    );
                }
            }
        }
        output
    }

    async fn handle_request(
        &mut self,
        id: RequestId,
        tag: String,
        payload: Value,
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
    ) -> Vec<Value> {
        if !valid_request_id(id.as_str()) {
            return vec![connection_defect(
                "Request id is not a decimal string and cannot be correlated",
            )];
        }
        if self.streams.contains_key(id.as_str()) {
            return vec![connection_defect(format!(
                "Duplicate in-flight RPC request id: {id}"
            ))];
        }

        // The grant this deployment advertised, enforced. A client is told at
        // pairing which scopes it has; refusing here is what makes that
        // advertised list a description of the server rather than a wish.
        //
        // This is the deployment's grant, not the client's — see the plan's Out
        // of scope note. A pairing that asked for less than this still gets a
        // credential carrying all of it.
        let required = oauth::required_scope(&tag);
        if !oauth::GRANTED_SCOPES.contains(&required) {
            debug!(%tag, %required, user_id = %identity.user_id, "T3 UI request refused; scope not granted");
            return vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "EnvironmentAuthorizationError",
                    "message": format!("This server does not grant {required}."),
                    "requiredScope": required,
                }),
            )];
        }

        match tag.as_str() {
            "server.probe" => vec![success_exit(&id, json!({}))],
            "server.getConfig" => {
                vec![success_exit(&id, server_config(state, identity).await)]
            }
            "server.getSettings" => vec![success_exit(&id, json!({}))],
            "server.refreshProviders" => {
                let agents = state.agents.list(&viewer_scope(identity)).await;
                vec![success_exit(
                    &id,
                    json!({ "providers": providers::providers(&agents, &state.model_defaults, &now_rfc3339()) }),
                )]
            }
            // The one write. A refusal is a typed failure the composer renders,
            // not a defect: "that action does not exist here" is true, "this
            // server is broken" is not.
            "orchestration.dispatchCommand" => {
                let ctx = commands::CommandContext {
                    conversations: state.conversations.as_ref(),
                    agents: state.agents.as_ref(),
                    sink: state.commands.as_ref(),
                    workspaces: state.workspaces.as_ref(),
                };
                match commands::dispatch(&ctx, &viewer_scope(identity), &payload).await {
                    Ok(result) => vec![success_exit(&id, result)],
                    Err(refusal) => {
                        debug!(
                            %tag,
                            user_id = %identity.user_id,
                            reason = %refusal.message(),
                            "T3 UI command refused"
                        );
                        vec![typed_failure_exit(
                            &id,
                            json!({
                                "_tag": "OrchestrationDispatchCommandError",
                                "message": refusal.message(),
                            }),
                        )]
                    }
                }
            }
            "server.discoverSourceControl" => {
                let github = state
                    .source_control_auth
                    .github_identity(&viewer_scope(identity))
                    .await;
                vec![success_exit(&id, git_actions::discovery(github.as_ref()))]
            }
            "vcs.listRefs" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![success_exit(&id, review::no_refs())];
                };
                let request = review::RefRequest::from_payload(&cwd, &payload);
                // A draft thread has no Sandbox yet and addresses its Workspace
                // by the project root. Its branches are the primary
                // repository's, read from the git host — the same list the
                // Moatless branch picker shows before a Task runs.
                if projection::is_project_root(&cwd) {
                    return match review::workspace_branches(
                        state.workspaces.as_ref(),
                        state.branches.as_ref(),
                        &viewer_scope(identity),
                        &cwd,
                        &request,
                    )
                    .await
                    {
                        Ok(refs) => vec![success_exit(&id, refs)],
                        Err(error) => vec![branch_list_exit(&id, &cwd, error, identity)],
                    };
                }
                match review::list_refs(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &request,
                )
                .await
                {
                    Ok(refs) => vec![success_exit(&id, refs)],
                    Err(error) if review::refs_are_quiet(&error) => {
                        vec![success_exit(&id, review::no_refs())]
                    }
                    Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, identity)],
                }
            }
            "vcs.refreshStatus" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    // No cwd is why this failed, so there is none to report.
                    return vec![review_failure_exit(
                        &id,
                        &tag,
                        "",
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match review::status_result(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                )
                .await
                {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, identity)],
                }
            }
            "vcs.switchRef" => {
                // A draft thread has no checkout to switch: its branch is
                // recorded on the draft and applied when the Task is created,
                // so the switch is answered as done, echoing the ref the
                // client should store. A thread that already has a Sandbox
                // cannot change branch — that is fixed at creation — so the
                // switch there falls through to unsupported.
                match branch_switch_result(&payload) {
                    Some(result) => vec![success_exit(&id, result)],
                    None => vec![unsupported_exit(&id, &tag)],
                }
            }
            "review.getDiffPreview" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    // No cwd is why this failed, so there is none to report.
                    return vec![review_failure_exit(
                        &id,
                        &tag,
                        "",
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match review::diff_preview(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                    payload
                        .get("baseRef")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|base| !base.is_empty()),
                    payload
                        .get("ignoreWhitespace")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
                .await
                {
                    Ok(preview) => vec![success_exit(&id, preview)],
                    Err(error) => vec![review_failure_exit(&id, &tag, &cwd, &error, identity)],
                }
            }
            "terminal.open" => {
                let viewer = viewer_scope(identity);
                match terminal::open(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(snapshot) => vec![success_exit(&id, snapshot)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.attach" => {
                let viewer = viewer_scope(identity);
                let stream = match terminal::attach(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(stream) => stream,
                    Err(error) => {
                        return vec![terminal_failure_exit(&id, &payload, &error, identity)];
                    }
                };
                let producer = tokio::spawn(terminal::forward_attach_stream(
                    stream,
                    state.terminal.clone(),
                    id.clone(),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, None, producer)
            }
            "terminal.write" => {
                let viewer = viewer_scope(identity);
                match terminal::write(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.resize" => {
                let viewer = viewer_scope(identity);
                match terminal::resize(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.clear" => {
                let viewer = viewer_scope(identity);
                match terminal::clear(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.restart" => {
                let viewer = viewer_scope(identity);
                match terminal::restart(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(snapshot) => vec![success_exit(&id, snapshot)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "terminal.close" => {
                let viewer = viewer_scope(identity);
                match terminal::close(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(error) => vec![terminal_failure_exit(&id, &payload, &error, identity)],
                }
            }
            "preview.list" => preview_answer(
                &id,
                preview::list(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            "preview.open" => preview_answer(
                &id,
                preview::open(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            "preview.navigate" => preview_answer(
                &id,
                preview::navigate(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            "preview.resize" => preview_answer(
                &id,
                preview::resize(&state.preview, &viewer_scope(identity), &payload).await,
            ),
            // The three void methods answer `null` rather than a snapshot,
            // matching the contract's declared success type.
            "preview.refresh" => preview_answer(
                &id,
                preview::refresh(&state.preview, &viewer_scope(identity), &payload)
                    .await
                    .map(|()| Value::Null),
            ),
            "preview.reportStatus" => preview_answer(
                &id,
                preview::report_status(&state.preview, &viewer_scope(identity), &payload)
                    .await
                    .map(|()| Value::Null),
            ),
            "preview.close" => preview_answer(
                &id,
                preview::close(&state.preview, &viewer_scope(identity), &payload)
                    .await
                    .map(|()| Value::Null),
            ),
            "servers.list" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "servers.list requires a threadId")];
                };
                match servers::list(&state.servers, &viewer, task_id).await {
                    Ok(listing) => vec![success_exit(&id, listing)],
                    // See [`servers_not_found`]: an unprovisioned conversation
                    // has no servers, which is an answer and not a failure.
                    Err(ServersError::NotFound) => vec![success_exit(&id, servers_not_found())],
                    Err(error) => vec![servers_failure_exit(&id, &error, identity)],
                }
            }
            "subtasks.list" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "subtasks.list requires a threadId")];
                };
                match subtasks::list(&state.subtasks, &viewer, task_id).await {
                    Ok(listing) => vec![success_exit(&id, listing)],
                    // A Thread the viewer cannot read has no subtasks *for
                    // them*, and the panel is a section on a page they are
                    // already looking at: an empty list is the honest answer
                    // and a failure would red-flag a working page.
                    Err(SubtasksError::NotFound) => {
                        vec![success_exit(&id, json!({ "subtasks": [] }))]
                    }
                    Err(error) => vec![subtasks_failure_exit(&id, &error, identity)],
                }
            }
            "scripts.run" => {
                let viewer = viewer_scope(identity);
                match scripts::run(
                    &state.scripts,
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &state.terminal,
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(result) => vec![success_exit(&id, result)],
                    Err(error) => vec![scripts_failure_exit(&id, &error, &payload, identity)],
                }
            }
            "sandbox.status" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "sandbox.status requires a threadId")];
                };
                match sandbox::status(&state.sandbox, &viewer, task_id).await {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![sandbox_failure_exit(&id, &error, identity)],
                }
            }
            "sandbox.start" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "sandbox.start requires a threadId")];
                };
                match sandbox::start(&state.sandbox, &viewer, task_id).await {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![sandbox_failure_exit(&id, &error, identity)],
                }
            }
            "sandbox.stop" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "sandbox.stop requires a threadId")];
                };
                match sandbox::stop(&state.sandbox, &viewer, task_id).await {
                    Ok(status) => vec![success_exit(&id, status)],
                    Err(error) => vec![sandbox_failure_exit(&id, &error, identity)],
                }
            }
            "sandbox.subscribeStatus" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(
                        &id,
                        "sandbox.subscribeStatus requires a threadId",
                    )];
                };
                let task_id = task_id.to_string();
                // Seed from the same read the poller makes, so the indicator
                // has a status before the first interval elapses rather than
                // after it.
                let seeded = match sandbox::status(&state.sandbox, &viewer, &task_id).await {
                    Ok(status) => status,
                    Err(error) => return vec![sandbox_failure_exit(&id, &error, identity)],
                };
                let producer = tokio::spawn(sandbox::watch_status(
                    id.clone(),
                    state.sandbox.clone(),
                    viewer,
                    task_id,
                    seeded.clone(),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, Some(vec![seeded]), producer)
            }
            "sandbox.listDiagnostics" => match sandbox::list_diagnostics(&state.sandbox).await {
                Ok(entries) => vec![success_exit(&id, entries)],
                Err(error) => vec![sandbox_failure_exit(&id, &error, identity)],
            },
            "subscribeServerStatus" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(
                        &id,
                        "subscribeServerStatus requires a threadId",
                    )];
                };
                let task_id = task_id.to_string();
                // Seed from the same read the poller makes, so the panel has a
                // list before the first interval elapses rather than after it.
                let seeded = match servers::list(&state.servers, &viewer, &task_id).await {
                    Ok(listing) => json!({
                        "threadId": task_id,
                        "servers": listing.get("servers").cloned().unwrap_or(json!([])),
                        "observedAt": chrono::Utc::now().to_rfc3339(),
                    }),
                    Err(ServersError::NotFound) => json!({
                        "threadId": task_id,
                        "servers": [],
                        "observedAt": chrono::Utc::now().to_rfc3339(),
                    }),
                    Err(error) => return vec![servers_failure_exit(&id, &error, identity)],
                };
                let producer = tokio::spawn(servers::watch_status(
                    id.clone(),
                    state.servers.clone(),
                    viewer,
                    task_id,
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, Some(vec![seeded]), producer)
            }
            "servers.subscribeLogs" => {
                let viewer = viewer_scope(identity);
                let (Some(task_id), Some(name)) = (
                    payload.get("threadId").and_then(Value::as_str),
                    payload.get("name").and_then(Value::as_str),
                ) else {
                    return vec![failure_exit(
                        &id,
                        "servers.subscribeLogs requires a threadId and a name",
                    )];
                };
                let producer = tokio::spawn(servers::watch_logs(
                    id.clone(),
                    state.servers.clone(),
                    viewer,
                    task_id.to_string(),
                    name.to_string(),
                    payload
                        .get("previous")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, None, producer)
            }
            "projects.listEntries" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match files::list_entries(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                )
                .await
                {
                    Ok(listing) => vec![success_exit(&id, listing)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "projects.readFile" => {
                let relative_path = payload
                    .get("relativePath")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty());
                let (Some(cwd), Some(relative_path)) = (workspace_cwd(&payload), relative_path)
                else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match files::read_file(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                    relative_path,
                )
                .await
                {
                    Ok(file) => vec![success_exit(&id, file)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "projects.searchEntries" => {
                // An absent or blank query is legal and means "show me what is
                // here": the picker opens before anyone types. `search_entries`
                // answers that with a bounded listing rather than nothing,
                // because the Sandbox's name search matches no path at all for
                // an empty needle and an empty picker looks like a broken one.
                let query = payload
                    .get("query")
                    .and_then(Value::as_str)
                    .map_or("", str::trim);
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                let limit = payload
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map_or(SEARCH_ENTRIES_LIMIT, |limit| {
                        (limit as usize).min(SEARCH_ENTRIES_MAX)
                    });
                match files::search_entries(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                    query,
                    limit,
                    payload.get("kind").and_then(Value::as_str),
                )
                .await
                {
                    Ok(entries) => vec![success_exit(&id, entries)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "projects.writeFile" => {
                let relative_path = payload
                    .get("relativePath")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty());
                // Empty contents are a legal write — truncating a file is a
                // thing a person does — so only an absent `contents` is
                // unaddressable.
                let contents = payload.get("contents").and_then(Value::as_str);
                let (Some(cwd), Some(relative_path), Some(contents)) =
                    (workspace_cwd(&payload), relative_path, contents)
                else {
                    return vec![file_failure_exit(
                        &id,
                        &tag,
                        &WorkspaceAccessError::Unaddressable,
                        identity,
                    )];
                };
                match files::write_file(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &cwd,
                    relative_path,
                    contents,
                )
                .await
                {
                    Ok(written) => vec![success_exit(&id, written)],
                    Err(error) => vec![file_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "orchestration.searchThreads" => {
                let query = payload
                    .get("query")
                    .and_then(Value::as_str)
                    .map_or("", str::trim);
                // See [`SEARCH_THREADS_MIN_QUERY`]. Answering an empty list is
                // the honest response to "match anything", and it costs nothing.
                if query.chars().count() < SEARCH_THREADS_MIN_QUERY {
                    return vec![success_exit(&id, json!({ "matches": [] }))];
                }
                let limit = payload
                    .get("limit")
                    .and_then(Value::as_i64)
                    .filter(|limit| *limit > 0)
                    .map_or(SEARCH_THREADS_LIMIT, |limit| {
                        limit.min(SEARCH_THREADS_LIMIT)
                    });
                match state
                    .conversations
                    .search_conversations(&viewer_scope(identity), query, limit)
                    .await
                {
                    Ok(matches) => vec![success_exit(
                        &id,
                        json!({ "matches": projection::thread_matches(&matches, query) }),
                    )],
                    Err(error) => {
                        error!(%error, user_id = %identity.user_id, "T3 UI thread search failed");
                        vec![typed_failure_exit(
                            &id,
                            json!({
                                "_tag": "OrchestrationSearchThreadsError",
                                "message": "Failed to search conversations.",
                            }),
                        )]
                    }
                }
            }
            "cloud.getRelayClientStatus" => vec![success_exit(
                &id,
                json!({
                    "status": "unsupported",
                    "platform": platform_os(),
                    "arch": platform_arch(),
                    "version": T3_CLIENT_CONTRACT_VERSION
                }),
            )],
            // The listings' complement: one row, by id, for a conversation the
            // client is holding and no listing named. `null` is the answer for
            // every way it can be absent — the client is already on that page,
            // and a failure would take a rendered transcript off the screen to
            // report a missing title.
            "threads.getShell" => {
                let viewer = viewer_scope(identity);
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    return vec![failure_exit(&id, "threads.getShell requires a threadId")];
                };
                match thread_shell(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                    task_id,
                )
                .await
                {
                    Ok(thread) => vec![success_exit(
                        &id,
                        json!({ "thread": thread.unwrap_or(Value::Null) }),
                    )],
                    Err(error) => vec![shell_failure_exit(&id, &error, identity)],
                }
            }
            "orchestration.getArchivedShellSnapshot" => {
                let viewer = viewer_scope(identity);
                match archived_shell_snapshot(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                )
                .await
                {
                    Ok(snapshot) => vec![success_exit(&id, snapshot)],
                    Err(error) => vec![shell_failure_exit(&id, &error, identity)],
                }
            }
            // Still answers with nothing, and deliberately. This is not the
            // path a reconnecting client takes: no T3 client calls it — web,
            // desktop and mobile all share `client-runtime`, whose only resume
            // is `subscribeShell` with `afterSequence` (`state/shell.ts:181`),
            // and `replayEvents` appears outside `packages/contracts` only in
            // the reference server and its own tests. Its result type is
            // `OrchestrationEvent`, which carries an event id, an aggregate, a
            // causation chain and a typed payload; a listing change record
            // carries a position and a Task id and nothing else, on purpose.
            // Filling this in would mean inventing an event log this backend
            // does not have, for a method nothing calls.
            "orchestration.replayEvents" => vec![success_exit(&id, json!([]))],
            "orchestration.subscribeShell" => {
                let viewer = viewer_scope(identity);
                let subscription = match shell_subscription(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                    &state.environment,
                    after_sequence(&payload),
                )
                .await
                {
                    Ok(subscription) => subscription,
                    Err(error) => return vec![shell_failure_exit(&id, &error, identity)],
                };
                // Both halves come from `shell_subscription`: the items to send,
                // and a feed that already knows what sending them leaves the
                // client holding.
                let mut values = subscription.values;
                if payload
                    .get("requestCompletionMarker")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    // Last, after the snapshot or the catch-up. The contract
                    // places the marker "after the subscription has emitted its
                    // initial snapshot or catch-up replay and before it begins
                    // emitting live events", and the client reads it as
                    // permission to stop rendering itself as synchronizing.
                    values.push(json!({ "kind": "synchronized" }));
                }
                let frames = self.frames.clone();
                let producer = tokio::spawn(Self::watch_shell(
                    id.clone(),
                    subscription.feed,
                    state.clone(),
                    viewer,
                    frames,
                ));
                self.start_produced_stream(&id, Some(values), producer)
            }
            "orchestration.subscribeThread" => {
                let Some(task_id) = payload.get("threadId").and_then(Value::as_str) else {
                    // The contract requires it and every client encodes against
                    // the contract, so this is a malformed request rather than
                    // a conversation that is missing.
                    return vec![failure_exit(
                        &id,
                        "orchestration.subscribeThread requires a threadId",
                    )];
                };
                let task_id = task_id.to_string();
                let viewer = viewer_scope(identity);
                let opened = match thread_subscription(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.deployment(),
                    &viewer,
                    &task_id,
                    after_sequence(&payload),
                    payload
                        .get("requestCompletionMarker")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
                .await
                {
                    Ok(opened) => opened,
                    Err(error) => {
                        error!(
                            %error,
                            user_id = %identity.user_id,
                            task_id = %task_id,
                            "Failed to assemble T3 UI thread snapshot"
                        );
                        return vec![failure_exit(&id, "Failed to assemble the conversation")];
                    }
                };
                let Some((values, feed)) = opened else {
                    return vec![failure_exit(&id, "Conversation not found")];
                };
                let frames = self.frames.clone();
                let producer = tokio::spawn(Self::watch_thread(
                    id.clone(),
                    feed,
                    state.clone(),
                    viewer,
                    frames,
                ));
                self.start_produced_stream(&id, Some(values), producer)
            }
            // A conversation's checkout exists inside its Sandbox and nowhere
            // else, so an asset is minted as a route this backend serves the
            // file on rather than as a path on this machine — see [`assets`].
            // An attachment is an upload and is minted at the upload route. A
            // project's favicon names no file here and keeps answering that
            // there is none, which is what the client renders as a project
            // without an icon.
            "assets.createUrl" => {
                // The error carries the resource back, and the client decodes it
                // against the same schema it sent — so a request without one has
                // no typed answer to give and is a malformed request instead.
                let Some(resource) = payload.get("resource") else {
                    return vec![failure_exit(&id, "assets.createUrl requires a resource")];
                };
                match assets::create_url(resource) {
                    Ok(minted) => vec![success_exit(&id, minted)],
                    Err(error) => vec![typed_failure_exit(
                        &id,
                        json!({
                            "_tag": error.tag(),
                            "resource": resource
                        }),
                    )],
                }
            }
            // Where the composer sends a file before it sends the turn naming
            // it. The URL carries its own credential and the route serves it
            // — see [`attachments`] — so nothing about the upload reaches this
            // socket except the id it will be known by.
            "attachments.createUploadUrl" => {
                match attachments::create_upload_url(
                    identity,
                    state.session_jwt_secret.as_deref(),
                    &payload,
                ) {
                    Ok(minted) => vec![success_exit(&id, minted)],
                    Err(attachments::MintError::Invalid(message)) => {
                        vec![failure_exit(&id, message)]
                    }
                    Err(attachments::MintError::NoSigningKey) => vec![typed_failure_exit(
                        &id,
                        json!({
                            "_tag": "AttachmentUploadSigningKeyError",
                            "cause": "This deployment cannot sign attachment upload URLs."
                        }),
                    )],
                }
            }
            "attachments.delete" => {
                match attachments::delete(
                    state.attachments.as_ref(),
                    &viewer_scope(identity),
                    &payload,
                )
                .await
                {
                    Ok(()) => vec![success_exit(&id, Value::Null)],
                    Err(message) => vec![failure_exit(&id, message)],
                }
            }
            // Which client is focused decides whether *T3* keeps background
            // work running on its own host. Moatless runs its agents in
            // Sandboxes that no browser tab is attached to, so the report is
            // accepted and dropped rather than refused: the client sends it on
            // every focus change and a defect per change is pure noise.
            "server.reportClientActivity" => vec![success_exit(&id, Value::Null)],
            "subscribeServerConfig" => self.start_stream(
                &id,
                Some(vec![json!({
                    "version": 1,
                    "type": "snapshot",
                    "config": server_config(state, identity).await
                })]),
            ),
            "subscribeServerLifecycle" => self.start_stream(
                &id,
                Some(vec![json!({
                    "version": 1,
                    "sequence": 1,
                    "type": "welcome",
                    "payload": {
                        "environment": descriptor(&state.rpc),
                        "cwd": "/moatless",
                        "projectName": T3_ENVIRONMENT_LABEL
                    }
                })]),
            ),
            "subscribeAuthAccess" => self.start_stream(
                &id,
                Some(vec![json!({
                    "version": 1,
                    "revision": 1,
                    "type": "snapshot",
                    "payload": {
                        "pairingLinks": [],
                        "clientSessions": []
                    }
                })]),
            ),
            "subscribePreviewEvents" => {
                let producer = tokio::spawn(Self::watch_preview_events(
                    id.clone(),
                    state.clone(),
                    viewer_scope(identity),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, None, producer)
            }
            "subscribeTerminalEvents" => {
                let producer = tokio::spawn(terminal::forward_value_subscription(
                    state.terminal.subscribe_events(),
                    id.clone(),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, None, producer)
            }
            "subscribeTerminalMetadata" => {
                let snapshot = state.terminal.metadata_snapshot().await;
                let producer = tokio::spawn(terminal::forward_value_subscription(
                    state.terminal.subscribe_metadata(),
                    id.clone(),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, Some(vec![snapshot]), producer)
            }
            "subscribeVcsStatus" => {
                // A `cwd` this surface does not address is not a failure here:
                // the client opens this subscription for every thread it shows,
                // including ones with no sandbox, and a stream that says nothing
                // is what "there is no repository to report on" looks like.
                let Some(cwd) = workspace_cwd(&payload) else {
                    return self.start_stream(&id, None);
                };
                let producer = tokio::spawn(Self::watch_vcs_status(
                    id.clone(),
                    cwd,
                    state.clone(),
                    viewer_scope(identity),
                    self.frames.clone(),
                ));
                self.start_produced_stream(&id, None, producer)
            }
            "git.runStackedAction" => {
                // The action is not run here or in the Sandbox: the button press
                // becomes a message asking the Task's agent to do the git work.
                // The client's contract is a stream, so it is answered with one
                // `action_finished` chunk and a terminal success, the same shape
                // a completed action produced.
                let viewer = viewer_scope(identity);
                match git_actions::run_stacked_action(
                    state.conversations.as_ref(),
                    state.commands.as_ref(),
                    &viewer,
                    &payload,
                )
                .await
                {
                    Ok(sent) => vec![
                        json!({
                            "_tag": "Chunk",
                            "requestId": id.wire(),
                            "values": [git_actions::asked_agent_event(&sent)],
                        }),
                        success_exit(&id, Value::Null),
                    ],
                    Err(error) => vec![git_action_failure_exit(&id, &error, identity)],
                }
            }
            "git.resolvePullRequest" => {
                match git_actions::resolve_pull_request(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
                    &payload,
                )
                .await
                {
                    Ok(result) => vec![success_exit(&id, result)],
                    Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "git.preparePullRequestThread" => {
                match git_actions::prepare_pull_request_thread(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    state.pull_requests.as_ref(),
                    &viewer_scope(identity),
                    &payload,
                )
                .await
                {
                    Ok(git_actions::PreparedThread::Local(result)) => {
                        vec![success_exit(&id, result)]
                    }
                    // Worktree mode is refused by name, not by falling through to
                    // the catch-all: this surface prepares a branch, never a
                    // worktree on disk.
                    Ok(git_actions::PreparedThread::WorktreeUnsupported) => {
                        vec![unsupported_exit(&id, &tag)]
                    }
                    Err(error) => vec![pull_request_failure_exit(&id, &tag, &error, identity)],
                }
            }
            // `subscribeDiscoveredLocalServers` falls through here and says so:
            // there is no local-server discovery, and a stream that never says
            // anything reads to a client as "connected, nothing to report".
            _ => vec![unsupported_exit(&id, &tag)],
        }
    }

    /// Report one conversation's branch state until its socket goes away.
    ///
    /// Polled rather than pushed: the state lives in a Sandbox's checkout,
    /// which changes when an agent writes files and tells nobody. The interval
    /// is long because of what reads it — the panel picks its opening scope
    /// from `hasWorkingTreeChanges` and then reads patches through
    /// `review.getDiffPreview`, which the client refetches on its own.
    ///
    /// A pass that cannot reach the Sandbox says nothing and waits for the next
    /// one. Ending the stream would present a stopped or still-starting sandbox
    /// — the ordinary state of most conversations — as a permanent answer the
    /// client would then hold until it reconnects.
    async fn watch_vcs_status(
        request_id: RequestId,
        cwd: String,
        state: UiRpcState,
        viewer: ScopeContext,
        frames: mpsc::Sender<Value>,
    ) {
        let mut last_sent: Option<Value> = None;
        loop {
            if let Ok(snapshot) = review::status(
                state.conversations.as_ref(),
                state.sandboxes.as_ref(),
                state.pull_requests.as_ref(),
                &viewer,
                &cwd,
            )
            .await
            {
                // Re-sending an unchanged snapshot costs the client a rerender
                // of every surface that reads git status, so only a change goes
                // on the wire.
                if last_sent.as_ref() != Some(&snapshot) {
                    let chunk = json!({
                        "_tag": "Chunk",
                        "requestId": request_id.wire(),
                        "values": [snapshot.clone()]
                    });
                    if frames.send(chunk).await.is_err() {
                        return;
                    }
                    last_sent = Some(snapshot);
                }
            }
            tokio::time::sleep(VCS_STATUS_POLL_INTERVAL).await;
        }
    }

    /// Watch the listing feed for one subscription until its socket goes away.
    ///
    /// Each pass delivers at most one chunk, however many changes it read: a
    /// burst on one conversation is coalesced into one re-read of that row, and
    /// the poll interval is the window that does the coalescing.
    ///
    /// Chunks are not gated on the client's `Ack`, which the reference server
    /// uses for backpressure. The client acks a chunk as soon as it has queued
    /// its values (`RpcClient`'s `Chunk` case offers to the queue, then acks),
    /// so it does not require the server to wait — and the poll interval already
    /// bounds this stream to four chunks a second regardless of what changed.
    ///
    /// The feed arrives built by [`shell_subscription`] — positioned at the
    /// snapshot's place or at wherever the catch-up read reached, and holding
    /// whatever that left the client holding. Both are facts about how the
    /// subscription opened and neither is this loop's to decide, which is also
    /// why nothing between the first chunk and the first live poll can fall
    /// through the join: it is the same feed either way.
    async fn watch_shell(
        request_id: RequestId,
        mut feed: ShellFeed,
        state: UiRpcState,
        viewer: ScopeContext,
        frames: mpsc::Sender<Value>,
    ) {
        loop {
            tokio::time::sleep(SHELL_POLL_INTERVAL).await;
            let values = match feed
                .poll(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
                    &state.model_defaults,
                    &viewer,
                )
                .await
            {
                Ok(values) => values,
                Err(error) => {
                    // The subscription survives a failed poll. Ending it would
                    // present a transient database error as a dead sidebar, and
                    // the cursor has not moved, so the next poll covers the same
                    // ground.
                    //
                    // That second clause is a fact about [`ShellFeed::poll`],
                    // not about this loop: the cursor advances last, after the
                    // reads that build the batch, so a failure in any of them
                    // does not skip it silently. Do not move it back.
                    warn!(
                        %error,
                        user_id = %viewer.user_id,
                        "Failed to read the T3 UI shell change feed"
                    );
                    continue;
                }
            };
            if values.is_empty() {
                continue;
            }
            let chunk = json!({
                "_tag": "Chunk",
                "requestId": request_id.wire(),
                "values": values
            });
            // A chunk that cannot be queued is not the poll's problem, and the
            // cursor is deliberately left where the poll put it: the receiver
            // is held by the `ProtocolSession`, so a closed channel means the
            // socket loop has already returned and this subscription is over —
            // there is no next poll for a rewound cursor to serve. What the
            // client resumes from is the `afterSequence` of the snapshot it
            // re-fetches over HTTP before resubscribing, not anything this feed
            // remembers. Note that the converse also holds: a chunk queued
            // successfully may still fail on the wire in `send_frames`, so
            // treating the queue as delivery confirmation would be wrong in the
            // other direction too.
            if frames.send(chunk).await.is_err() {
                return;
            }
        }
    }

    /// Push one conversation's events for as long as the subscription lives.
    ///
    /// The shape mirrors [`Self::watch_shell`] over a change log of its own, and
    /// runs at a quarter of the rate: a conversation records a position per
    /// Message block where the shell records one per Task, so a poll here reads
    /// a burst rather than a row.
    ///
    /// A poll has a third answer the shell's cannot produce: `None`, meaning the
    /// conversation is no longer readable — deleted, unfollowed, or its project
    /// ungrouped. The subscription cannot recover from any of those by polling
    /// again, so it ends rather than idling silently on a screen that would keep
    /// rendering the last snapshot as current.
    async fn watch_thread(
        request_id: RequestId,
        mut feed: ThreadFeed,
        state: UiRpcState,
        viewer: ScopeContext,
        frames: mpsc::Sender<Value>,
    ) {
        loop {
            tokio::time::sleep(THREAD_POLL_INTERVAL).await;
            let polled = feed
                .poll(state.conversations.as_ref(), &viewer, &state.frontend_url)
                .await;
            let values = match polled {
                Ok(Some(values)) => values,
                Ok(None) => {
                    let _ = frames
                        .send(failure_exit(&request_id, "Conversation not found"))
                        .await;
                    return;
                }
                Err(error) => {
                    // As in `watch_shell`: a failed poll leaves the cursor where
                    // it was, so the next one covers the same ground and the
                    // subscription survives a transient database error.
                    warn!(
                        %error,
                        user_id = %viewer.user_id,
                        "Failed to poll the T3 UI conversation"
                    );
                    continue;
                }
            };
            if values.is_empty() {
                continue;
            }
            let chunk = json!({
                "_tag": "Chunk",
                "requestId": request_id.wire(),
                "values": values
            });
            if frames.send(chunk).await.is_err() {
                return;
            }
        }
    }

    /// Send every preview event the viewer is allowed to see.
    ///
    /// The subscription is environment-wide — the contract's payload is empty
    /// and the client filters by thread itself — so this is the one place a
    /// tab opened on a Task the viewer cannot read would otherwise reach them.
    /// Each Task is authorized once per subscription and the answer is
    /// remembered: tab events are rare, and a scoped read per event would put
    /// a database round trip on a fan-out path.
    async fn watch_preview_events(
        request_id: RequestId,
        state: UiRpcState,
        viewer: ScopeContext,
        frames: mpsc::Sender<Value>,
    ) {
        let mut events = state.preview.tabs.subscribe();
        let mut readable: HashMap<String, bool> = HashMap::new();
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                // Lagged: this subscriber fell behind and lost events. It is
                // not a reason to end the stream — the client reconciles
                // against `preview.list` on the next event it does see.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    warn!(missed, user_id = %viewer.user_id, "A preview event subscriber lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let Some(task_id) = event.get("threadId").and_then(Value::as_str) else {
                continue;
            };
            let allowed = if let Some(allowed) = readable.get(task_id) {
                *allowed
            } else {
                let allowed = match state.preview.tasks.find_by_id(task_id, &viewer).await {
                    Ok(task) => task.is_some(),
                    Err(error) => {
                        warn!(%error, task_id, "Could not authorize a preview event");
                        continue;
                    }
                };
                readable.insert(task_id.to_string(), allowed);
                allowed
            };
            if !allowed {
                continue;
            }
            let chunk = json!({
                "_tag": "Chunk",
                "requestId": request_id.wire(),
                "values": [event]
            });
            if frames.send(chunk).await.is_err() {
                return;
            }
        }
    }

    /// Register a stream and hand it the task that produces its frames.
    ///
    /// The two are registered together because the stream limit can refuse the
    /// registration, and a producer nothing will read has to be aborted rather
    /// than left polling on behalf of nobody.
    fn start_produced_stream(
        &mut self,
        id: &RequestId,
        values: Option<Vec<Value>>,
        producer: JoinHandle<()>,
    ) -> Vec<Value> {
        let started = self.start_stream(id, values);
        match self.streams.get_mut(id.as_str()) {
            Some(stream) => stream.producer = Some(producer),
            None => producer.abort(),
        }
        started
    }

    fn start_stream(&mut self, id: &RequestId, values: Option<Vec<Value>>) -> Vec<Value> {
        if self.streams.len() >= MAX_ACTIVE_STREAMS {
            return vec![failure_exit(
                id,
                format!("At most {MAX_ACTIVE_STREAMS} RPC streams may be active"),
            )];
        }

        let awaiting_ack = values.is_some();
        self.streams.insert(
            id.as_str().to_string(),
            StreamState {
                awaiting_ack,
                producer: None,
            },
        );
        values
            .map(|values| {
                vec![json!({
                    "_tag": "Chunk",
                    "requestId": id.wire(),
                    "values": values
                })]
            })
            .unwrap_or_default()
    }
}

fn valid_request_id(id: &str) -> bool {
    let digits = id.strip_prefix('-').unwrap_or(id);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn descriptor(config: &UiRpcConfig) -> Value {
    let mut descriptor = json!({
        "environmentId": config.environment_id,
        "label": T3_ENVIRONMENT_LABEL,
        "platform": {
            "os": platform_os(),
            "arch": platform_arch()
        },
        "serverVersion": T3_CLIENT_CONTRACT_VERSION,
        "capabilities": {
            "repositoryIdentity": false,
            "connectionProbe": true,
            "threadSettlement": true,
            "threadSnooze": true,
            // Whether `thread.user-input.respond` is understood. A client
            // cannot otherwise tell this server from one that refuses it, and
            // the descriptor is read before any thread is opened.
            "userInputResponse": true,
            // Whether `sandbox.listDiagnostics` is understood. A client
            // cannot otherwise tell this server from one that answers
            // `UnsupportedMethodError`.
            "sandboxDiagnostics": true,
            // Whether `sandbox.subscribeStatus` is understood. Absent, a client
            // has to poll `sandbox.status` to notice a sandbox that started
            // after its panel opened, so this is what lets it stop.
            "sandboxStatusPush": true,
            // Whether `sandbox.status` reports `agentStatus` alongside the
            // lifecycle state. Absent, a client cannot tell "this server does
            // not say" from "this agent is idle", so the panel has nothing to
            // show; present, the key is there whenever the sandbox is up.
            "sandboxAgentStatus": true,
            // Whether `scripts.run` is understood. A workspace's scripts are
            // still listed on the project without this — they are read out of
            // the workspace either way — but a client only shows the control
            // that runs one where the server can actually run it. T3's own
            // server omits this key and answers `scripts.run` with
            // `UnsupportedMethodError`.
            "workspaceScripts": true
        }
    });

    // Both keys are `optionalKey` in the contract, and both are absent rather
    // than false where uploads cannot be served: a `null` fails the client's
    // decode of the whole descriptor, which is the one thing it reads before it
    // can connect at all.
    //
    // `attachmentUploads` is what moves the composer off the inline data-URL
    // path — for images as well as files — so it is reported only where a URL
    // can actually be signed (see [`attachments`]). The inline path stays
    // served either way, for the clients that have not read it.
    if config.attachment_uploads {
        if let Some(capabilities) = descriptor["capabilities"].as_object_mut() {
            capabilities.insert("attachmentUploads".into(), json!(true));
            // What a turn may carry beyond images, and how large. Absent, a
            // client offers no file picker at all.
            capabilities.insert(
                "fileAttachments".into(),
                json!({ "maxUploadBytes": attachments::MAX_FILE_BYTES }),
            );
        }
    }

    descriptor
}

fn auth_descriptor(state: &UiRpcState) -> Value {
    json!({
        "policy": "remote-reachable",
        // A pairing code is T3's `one-time-token`, and this is where a client
        // reads that pairing is on offer at all: the mobile app shows the flow
        // for what the environment says it accepts.
        "bootstrapMethods": ["one-time-token"],
        "sessionMethods": ["browser-session-cookie", "bearer-access-token"],
        "sessionCookieName": state.session_cookie_name.as_ref()
    })
}

/// What a client is told about the session it is already holding.
///
/// `sessionMethod` reports the credential this request actually arrived with,
/// rather than the one the browser would have used, because a paired device
/// asking what it is holding should not be told it is holding a cookie.
fn session_state(state: &UiRpcState, method: Option<UiRpcAuthMethod>) -> Value {
    json!({
        "authenticated": true,
        "auth": auth_descriptor(state),
        "scopes": oauth::GRANTED_SCOPES,
        "sessionMethod": match method {
            Some(UiRpcAuthMethod::SessionCookie) | None => "browser-session-cookie",
            Some(UiRpcAuthMethod::ApiKey) | Some(UiRpcAuthMethod::WebSocketTicket) => "bearer-access-token",
        }
    })
}

/// Now, in the format the contract's `IsoDateTime` decodes.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn server_config(state: &UiRpcState, identity: &UiRpcAuthIdentity) -> Value {
    let agents = state.agents.list(&viewer_scope(identity)).await;
    json!({
        "environment": descriptor(&state.rpc),
        "auth": auth_descriptor(state),
        "cwd": "/moatless",
        "keybindingsConfigPath": "/moatless/keybindings.json",
        "keybindings": [],
        "issues": [],
        "providers": providers::providers(&agents, &state.model_defaults, &now_rfc3339()),
        "availableEditors": [],
        "observability": {
            "logsDirectoryPath": "/moatless/logs",
            "localTracingEnabled": false,
            "otlpTracesEnabled": state.rpc.otlp_traces_enabled,
            "otlpMetricsEnabled": false
        },
        "settings": {},
        "shellResumeCompletionMarker": true,
        "threadResumeCompletionMarker": true
    })
}

fn platform_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

fn platform_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "x64"
    } else {
        "other"
    }
}

fn success_exit(request_id: &RequestId, value: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Success",
            "value": value
        }
    })
}

fn failure_exit(request_id: &RequestId, defect: impl Into<String>) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Die",
                "defect": defect.into()
            }]
        }
    })
}

/// Fail a request with an error the client's schema knows how to decode.
///
/// [`failure_exit`] dies with a string, which every caller renders as an
/// unexpected server defect. A `Fail` cause carrying a tagged error is instead
/// matched against the method's declared error type, so the client handles it as
/// one of that method's ordinary outcomes. Use this where the failure is one the
/// contract anticipated and the string form where it is not.
fn typed_failure_exit(request_id: &RequestId, error: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Fail",
                "error": error
            }]
        }
    })
}

/// A workspace-file request the client is told about in its own terms.
///
/// This is one of the failures [`typed_failure_exit`] exists for, and the
/// difference from a `Die` is what the person sees. Each of these methods
/// declares a tagged error in the contract, and the client renders that
/// error's `message` in the file panel; a `Die` reaches the same panel as
/// nothing but *"Workspace query failed."*, because the defect never decodes
/// into an `Error` for it to read a message off
/// (`projectFilesQueryState.ts:errorMessage`). Refusing to open a file is a
/// normal outcome here — no sandbox, a binary file, a stopped pod — and each of
/// those is worth saying out loud.
///
/// The tag names which error type the method declares, since the client decodes
/// the cause against that method's own union and a mismatch would land back as
/// an undecodable defect. Causes stay in the log: the `message` is written for
/// the person, and the sqlx or gRPC failure behind it is not.
fn file_failure_exit(
    request_id: &RequestId,
    tag: &str,
    error: &WorkspaceAccessError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let error_tag = match tag {
        "projects.readFile" => "ProjectReadFileError",
        "projects.searchEntries" => "ProjectSearchEntriesError",
        "projects.writeFile" => "ProjectWriteFileError",
        _ => "ProjectListEntriesError",
    };
    match error {
        WorkspaceAccessError::Database(cause) => {
            error!(%cause, %tag, user_id = %identity.user_id, "T3 UI workspace file request failed");
        }
        WorkspaceAccessError::Sandbox(cause) => {
            warn!(%cause, %tag, user_id = %identity.user_id, "T3 UI workspace file request failed");
        }
        // A sandbox that is not up is the ordinary state of most conversations,
        // so this one is not a warning about the deployment.
        WorkspaceAccessError::NotRunning(cause) => {
            debug!(%cause, %tag, user_id = %identity.user_id, "T3 UI workspace file request refused");
        }
        WorkspaceAccessError::NoWorktree
        | WorkspaceAccessError::Unaddressable
        | WorkspaceAccessError::NoControl => {
            debug!(?error, %tag, user_id = %identity.user_id, "T3 UI workspace file request refused");
        }
    }
    typed_failure_exit(
        request_id,
        json!({ "_tag": error_tag, "message": error.message() }),
    )
}

/// Why a review request could not be answered, as the failure its method
/// declares.
///
/// `GitCommandError` is the one shape both methods' error unions carry, and the
/// client renders its `message` — `Git command failed in {operation} ({cwd}):
/// {detail}` — in the diff panel. A `Die` would reach the same panel as nothing
/// but an unexpected server error, and refusing to show a diff is a normal
/// outcome here: no sandbox, a stopped pod, a conversation this viewer cannot
/// be told about.
fn review_failure_exit(
    request_id: &RequestId,
    tag: &str,
    cwd: &str,
    error: &WorkspaceAccessError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    match error {
        WorkspaceAccessError::Database(cause) => {
            error!(%cause, %tag, user_id = %identity.user_id, "T3 UI review request failed");
        }
        WorkspaceAccessError::Sandbox(cause) => {
            warn!(%cause, %tag, user_id = %identity.user_id, "T3 UI review request failed");
        }
        // A sandbox that is not up is the ordinary state of most conversations,
        // so this one is not a warning about the deployment.
        WorkspaceAccessError::NotRunning(cause) => {
            debug!(%cause, %tag, user_id = %identity.user_id, "T3 UI review request refused");
        }
        WorkspaceAccessError::NoWorktree
        | WorkspaceAccessError::Unaddressable
        | WorkspaceAccessError::NoControl => {
            debug!(?error, %tag, user_id = %identity.user_id, "T3 UI review request refused");
        }
    }
    // `command` names what was actually attempted. Nothing here shells out to
    // git: the Sandbox answers these over gRPC, and reporting `git` as the
    // command sends anyone reading a support log looking for a shell invocation
    // that does not exist.
    let command = match tag {
        "vcs.listRefs" => "sandbox.ListGitRefs",
        "vcs.refreshStatus" => "sandbox.GetGitStatus",
        _ => "sandbox.GetGitDiffPreview",
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": tag,
            "command": command,
            "cwd": cwd,
            "detail": error.message(),
        }),
    )
}

/// Refuse a `git.runStackedAction` that could not be handed to the agent.
///
/// A refusal the viewer caused — an unaddressable `cwd`, a Task they do not
/// control, a malformed request — or the message failing to send after it was
/// accepted. A rejected request is a `Die` (the client sent something it should
/// not have); an access refusal or a failed send carries the same
/// `GitCommandError` shape the read path uses, so it renders in the toolbar the
/// same way a status read's failure does.
fn git_action_failure_exit(
    request_id: &RequestId,
    error: &git_actions::ActionError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let detail = match error {
        git_actions::ActionError::Rejected(message) => {
            debug!(%message, user_id = %identity.user_id, "T3 git action rejected");
            return failure_exit(request_id, message.clone());
        }
        git_actions::ActionError::Access(access) => {
            match access {
                WorkspaceAccessError::Database(cause) => {
                    error!(%cause, user_id = %identity.user_id, "T3 git action failed");
                }
                WorkspaceAccessError::Sandbox(cause) => {
                    warn!(%cause, user_id = %identity.user_id, "T3 git action failed");
                }
                WorkspaceAccessError::NotRunning(cause) => {
                    debug!(%cause, user_id = %identity.user_id, "T3 git action refused");
                }
                WorkspaceAccessError::NoWorktree
                | WorkspaceAccessError::Unaddressable
                | WorkspaceAccessError::NoControl => {
                    debug!(?access, user_id = %identity.user_id, "T3 git action refused");
                }
            }
            access.message()
        }
        git_actions::ActionError::Send(refusal) => {
            warn!(
                reason = %refusal.message(),
                user_id = %identity.user_id,
                "T3 git action could not ask the agent"
            );
            refusal.message()
        }
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": "git.runStackedAction",
            "command": "orchestration.sendMessage",
            "cwd": "",
            "detail": detail,
        }),
    )
}

/// Why a pull-request reference could not be resolved, as the failure both
/// dialog methods declare.
///
/// `GitManagerError` is one shape of their `GitManagerServiceError` union, and
/// the client renders its `message` — `Git manager failed in {operation}:
/// {detail}` — in the attach-PR dialog. A reference that names no reachable pull
/// request, or a repository the viewer cannot be shown, is an ordinary outcome
/// there; a database failure is logged and reported the same way, since a `Die`
/// would reach the dialog as an undecodable defect.
fn pull_request_failure_exit(
    request_id: &RequestId,
    tag: &str,
    error: &ports::PullRequestError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let detail = match error {
        ports::PullRequestError::NotFound => {
            "No pull request was found for that reference.".to_string()
        }
        ports::PullRequestError::Unavailable(message) => message.clone(),
        ports::PullRequestError::Failed(cause) => {
            error!(%cause, %tag, user_id = %identity.user_id, "T3 pull-request resolution failed");
            "The pull request could not be resolved.".to_string()
        }
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitManagerError",
            "operation": tag,
            // Nothing here shells out to git: the pull request is read from the
            // git host's API, so this names the read rather than a checkout.
            "cwd": "",
            "detail": detail,
        }),
    )
}

/// Answer a project-root `vcs.listRefs` whose branch listing did not produce a
/// page.
///
/// A workspace with no repository the viewer can be shown answers with an empty
/// list, the same as a conversation with no Sandbox. The other outcomes are the
/// branch picker's own: an unavailable git host — no token, a host that refused
/// — is surfaced verbatim so the "connect your account" prompt lands, and a
/// database failure is logged and reported as a generic git error, since
/// `GitCommandError` is the shape this method declares.
fn branch_list_exit(
    request_id: &RequestId,
    cwd: &str,
    error: ports::BranchListError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    let detail = match error {
        ports::BranchListError::None => return success_exit(request_id, review::no_refs()),
        ports::BranchListError::Unavailable(message) => message,
        ports::BranchListError::Failed(cause) => {
            error!(%cause, user_id = %identity.user_id, "T3 UI branch listing failed");
            "The branches could not be listed.".to_string()
        }
    };
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": "vcs.listRefs",
            // Nothing here shells out to git: the branches come from the git
            // host's API, not a checkout, so this names the read rather than a
            // shell invocation that does not exist.
            "command": "repository.ListBranches",
            "cwd": cwd,
            "detail": detail,
        }),
    )
}

/// The result of switching a draft thread to a branch, if this is one to
/// answer.
///
/// The switch is a no-op for a draft: there is no checkout yet, so the ref is
/// echoed back for the client to record on the draft and send at creation.
/// `None` — a switch on a thread that already has a Sandbox, or one with no
/// addressable `cwd` — is left for the caller to refuse, because a Task's
/// branch is fixed once its checkout exists.
fn branch_switch_result(payload: &Value) -> Option<Value> {
    let cwd = workspace_cwd(payload)?;
    if !projection::is_project_root(&cwd) {
        return None;
    }
    let ref_name = payload.get("refName").and_then(Value::as_str)?.trim();
    (!ref_name.is_empty()).then(|| json!({ "refName": ref_name }))
}

/// The client is told the listing could not be built; the cause is logged once
/// here rather than shipped to the browser.
fn shell_failure_exit(
    request_id: &RequestId,
    error: &sqlx::Error,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to assemble T3 UI shell snapshot");
    failure_exit(request_id, "Failed to assemble the shell listing")
}

/// Capacity and PostgreSQL statement cancellation are transient overload, not
/// malformed requests or permanent server defects. HTTP clients may retry them;
/// every other database error remains an internal failure.
fn shell_failure_status(error: &sqlx::Error) -> StatusCode {
    match error {
        sqlx::Error::PoolTimedOut => StatusCode::SERVICE_UNAVAILABLE,
        sqlx::Error::Database(error) if error.code().as_deref() == Some("57014") => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Answer a `preview.*` call, in the client's own terms where the contract
/// gives it terms.
///
/// An unknown tab is a `PreviewSessionLookupError` the panel renders; a store
/// that cannot be reached is a defect, because it is not one of the method's
/// declared outcomes and pretending otherwise would present an outage as a
/// missing tab.
fn preview_answer(request_id: &RequestId, result: Result<Value, PreviewError>) -> Vec<Value> {
    match result {
        Ok(value) => vec![success_exit(request_id, value)],
        Err(error) => {
            if let Some(typed) = preview::typed_error(&error) {
                vec![typed_failure_exit(request_id, typed)]
            } else {
                warn!(%error, "A T3 UI preview call failed");
                vec![failure_exit(request_id, error.to_string())]
            }
        }
    }
}

fn terminal_failure_exit(
    request_id: &RequestId,
    payload: &Value,
    error: &terminal::TerminalError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    match error {
        terminal::TerminalError::Workspace(WorkspaceAccessError::Database(cause)) => {
            error!(%cause, user_id = %identity.user_id, "T3 UI terminal request failed");
        }
        terminal::TerminalError::Workspace(WorkspaceAccessError::Sandbox(cause))
        | terminal::TerminalError::Workspace(WorkspaceAccessError::NotRunning(cause))
        | terminal::TerminalError::Sandbox(cause) => {
            warn!(%cause, user_id = %identity.user_id, "T3 UI terminal request failed");
        }
        terminal::TerminalError::Workspace(
            WorkspaceAccessError::NoWorktree
            | WorkspaceAccessError::Unaddressable
            | WorkspaceAccessError::NoControl,
        )
        | terminal::TerminalError::Malformed(_) => {
            debug!(?error, user_id = %identity.user_id, "T3 UI terminal request refused");
        }
    }
    match terminal::typed_error(error, payload) {
        Some(error) => typed_failure_exit(request_id, error),
        None => failure_exit(request_id, terminal::message(error)),
    }
}

/// The client is told the servers could not be read; the cause is logged once
/// here rather than shipped to the browser.
fn servers_failure_exit(
    request_id: &RequestId,
    error: &ServersError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to read a Task's servers");
    failure_exit(request_id, "Failed to read the conversation's servers")
}

/// The client is told the subtasks could not be read; the cause is logged once
/// here rather than shipped to the browser.
fn subtasks_failure_exit(
    request_id: &RequestId,
    error: &SubtasksError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to read a Task's subtasks");
    failure_exit(request_id, "Failed to read the conversation's subtasks")
}

/// How a failed `scripts.run` reaches the client.
///
/// A terminal failure is delegated to [`terminal_failure_exit`] so the person
/// sees the same typed "not running" they would from `terminal.open` directly.
/// A reserved-port refusal is a configuration mistake and its message is shown
/// verbatim; a database error is logged here and generalised.
fn scripts_failure_exit(
    request_id: &RequestId,
    error: &ScriptsError,
    payload: &Value,
    identity: &UiRpcAuthIdentity,
) -> Value {
    match error {
        ScriptsError::Terminal(terminal_error) => {
            // Reuse the terminal path's own logging and typed errors, addressed
            // to the session the script would have run in.
            let terminal_payload = json!({
                "threadId": payload.get("threadId").and_then(Value::as_str).unwrap_or_default(),
            });
            terminal_failure_exit(request_id, &terminal_payload, terminal_error, identity)
        }
        ScriptsError::Port(SandboxError::NotReady(_)) => {
            debug!(?error, user_id = %identity.user_id, "scripts.run refused: sandbox not running");
            // Typed, not a string: this is the one scripts.run refusal a client
            // can *act* on rather than report — start the Sandbox, wait for it,
            // and run the Script the person already asked for. A `Die` with a
            // sentence in it would leave that behind string matching.
            typed_failure_exit(
                request_id,
                json!({
                    "_tag": "SandboxNotRunningError",
                    "threadId": payload.get("threadId").and_then(Value::as_str).unwrap_or_default(),
                }),
            )
        }
        ScriptsError::Port(SandboxError::BadRequest(detail)) => {
            debug!(%detail, user_id = %identity.user_id, "scripts.run refused a reserved port");
            failure_exit(request_id, detail.clone())
        }
        ScriptsError::Port(cause) => {
            warn!(%cause, user_id = %identity.user_id, "scripts.run could not publish the port");
            failure_exit(request_id, "The script's port could not be published.")
        }
        ScriptsError::ScriptNotFound(name) => {
            debug!(%name, user_id = %identity.user_id, "scripts.run: no such script");
            failure_exit(request_id, format!("No script named \"{name}\"."))
        }
        ScriptsError::NotFound => {
            debug!(user_id = %identity.user_id, "scripts.run: conversation not found");
            failure_exit(request_id, "Conversation not found.")
        }
        ScriptsError::Malformed(field) => {
            failure_exit(request_id, format!("scripts.run requires {field}"))
        }
        ScriptsError::Database(cause) => {
            error!(%cause, user_id = %identity.user_id, "scripts.run failed to read the conversation");
            failure_exit(request_id, "Failed to read the conversation.")
        }
    }
}

/// How a failed `scripts.run`/list reaches an HTTP caller (`moat tasks
/// scripts`) — the same refusal vocabulary as [`scripts_failure_exit`], the
/// WS twin, as status codes instead of RPC exit shapes.
fn scripts_error_response(error: &ScriptsError) -> Response {
    match error {
        ScriptsError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Conversation not found." })),
        )
            .into_response(),
        ScriptsError::ScriptNotFound(name) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("No script named \"{name}\".") })),
        )
            .into_response(),
        ScriptsError::Malformed(field) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("scripts.run requires {field}") })),
        )
            .into_response(),
        // Typed with a `code`, not just a string: this is the one refusal a
        // caller can act on (start the sandbox and retry) rather than report.
        ScriptsError::Port(SandboxError::NotReady(_)) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Sandbox not running.", "code": "SANDBOX_NOT_RUNNING" })),
        )
            .into_response(),
        ScriptsError::Port(SandboxError::BadRequest(detail)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": detail.clone() })),
        )
            .into_response(),
        ScriptsError::Port(cause) => {
            warn!(%cause, "scripts.run could not publish the port");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "The script's port could not be published." })),
            )
                .into_response()
        }
        ScriptsError::Terminal(terminal_error) => {
            warn!(?terminal_error, "scripts.run's terminal could not be started");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "The script's terminal could not be started." })),
            )
                .into_response()
        }
        ScriptsError::Database(cause) => {
            error!(%cause, "scripts.run failed to read the conversation");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to read the conversation." })),
            )
                .into_response()
        }
    }
}

/// The client is told the sandbox status or command could not be answered; the
/// cause is logged once here rather than shipped to the browser.
fn sandbox_failure_exit(
    request_id: &RequestId,
    error: &SandboxRpcError,
    identity: &UiRpcAuthIdentity,
) -> Value {
    error!(%error, user_id = %identity.user_id, "Failed to answer a Task's sandbox lifecycle request");
    failure_exit(request_id, "Failed to read the conversation's sandbox")
}

/// A conversation with no Sandbox, as `servers.list` answers it.
///
/// `servers.list` declares no domain error, so there is no typed failure to give
/// here and a `Die` would reach the panel as an unexpected server error. The
/// server-list contract names this case itself: an empty list is what a project
/// with no sandbox has to list. Sandbox lifecycle status is answered separately
/// by `sandbox.status`.
fn servers_not_found() -> Value {
    json!({ "servers": [] })
}

/// The method is not one this server has.
///
/// Every RPC declares `UnsupportedMethodError`, so this decodes wherever it
/// lands and the client can say which method is absent, distinguishing a
/// server that does not implement a method from one that has failed.
fn unsupported_exit(request_id: &RequestId, tag: &str) -> Value {
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "UnsupportedMethodError",
            "method": tag,
            "message": format!("{tag} is not available on this server."),
        }),
    )
}

fn interrupted_exit(request_id: &RequestId) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Interrupt"
            }]
        }
    })
}

fn connection_defect(defect: impl Into<String>) -> Value {
    json!({
        "_tag": "Defect",
        "defect": defect.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dao::MockConversationDao;
    use crate::task_dao::{MockT3TaskDao, TaskView};
    use crate::workspace_dao::MockT3WorkspaceDao;
    use crate::listing::UiRpcListingHead;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use moatless_backend_core::sandbox::{LiveStatus, LogOptions, SandboxDesiredState, SandboxDiagnosticsContainer, SandboxDiagnosticsEntry, SandboxError, SandboxPortPublisher, SandboxStatus, ContractSandboxStatus, ServerLogStream};
    use moatless_types::task::TaskStatus;
    use moatless_identity::auth::pairing::{PairedDevice, PairingApiKeyIssuer, PairingCode, PairingCodeDao, PairingError, hash_key};
    use uuid::Uuid;

    /// The stub's database. Every read in these tests reports it, so nothing
    /// here exercises the identity guard — that needs two real databases, and
    /// is in `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
    fn stub_identity() -> UiRpcEnvironmentIdentity {
        UiRpcEnvironmentIdentity::new("7667539899605483559:16384")
    }

    /// A stopped Sandbox is the one `scripts.run` refusal the client answers by
    /// itself — it starts the Sandbox and runs the Script again — so it goes out
    /// as a `Fail` carrying `SandboxNotRunningError`, not a `Die` carrying a
    /// sentence. The tag and the `threadId` are the contract
    /// (`packages/contracts/src/sandbox.ts`); a `Die` here would put auto-start
    /// behind string matching.
    #[test]
    fn a_stopped_sandbox_refuses_a_script_run_in_terms_the_client_can_act_on() {
        let identity = UiRpcAuthIdentity {
            user_id: Uuid::nil(),
            login: "someone".to_string(),
            role: UserRole::User,
            provider: "test".to_string(),
            group_id: None,
        };
        let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
        let error = ScriptsError::Port(SandboxError::NotReady("stopped".to_string()));

        let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
        let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

        let cause = &exit["exit"]["cause"][0];
        assert_eq!(cause["_tag"], "Fail", "a Die would not decode client-side");
        assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
        assert_eq!(cause["error"]["threadId"], "task-1");
    }

    /// A reserved port is the person's configuration to fix, not the client's —
    /// it stays a plain refusal, so the typed arm above must not swallow every
    /// port failure.
    #[test]
    fn a_reserved_port_is_still_reported_rather_than_retried() {
        let identity = UiRpcAuthIdentity {
            user_id: Uuid::nil(),
            login: "someone".to_string(),
            role: UserRole::User,
            provider: "test".to_string(),
            group_id: None,
        };
        let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
        let error = ScriptsError::Port(SandboxError::BadRequest("Port 8001 is reserved".into()));

        let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
        let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

        assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
    }

    #[test]
    fn gate_parser_accepts_enabled_values() {
        for value in ["1", "true", "TRUE", "yes", "on", " on "] {
            assert!(parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
        }
    }

    #[test]
    fn gate_parser_accepts_disabled_values() {
        for value in ["0", "false", "FALSE", "no", "off", "", "  "] {
            assert!(!parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
        }
    }

    #[test]
    fn gate_parser_rejects_invalid_values() {
        let error = parse_enabled("T3_UI_RPC_ENABLED", "sometimes").unwrap_err();

        assert!(matches!(
            error,
            UiRpcConfigError::Invalid(message)
                if message == "T3_UI_RPC_ENABLED must be true or false"
        ));
    }

    /// A deployment whose Sandboxes are all unreachable.
    ///
    /// Every file, server and log request these tests make is refused before it
    /// would reach a Sandbox — by the `cwd`, by a listing that contains no
    /// conversations, or by a scoped Task read that finds nothing — so these
    /// seams exist to be held, not called. What a reachable Sandbox answers is
    /// a gRPC or K8s round trip and belongs in an integration test.
    struct UnreachableSandboxes;

    fn unreachable(task_id: &str) -> SandboxError {
        SandboxError::NotReady(task_id.to_string())
    }

    #[async_trait]
    impl SandboxConnector for UnreachableSandboxes {
        async fn sandbox_client(
            &self,
            task_id: &str,
        ) -> Result<sandbox_proto::client::SandboxClient, SandboxError> {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl SandboxIntentReader for UnreachableSandboxes {
        async fn desired_state(
            &self,
            task_id: &str,
        ) -> Result<moatless_types::sandbox::SandboxDesiredState, SandboxError> {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl SandboxPortPublisher for UnreachableSandboxes {
        async fn publish_port(
            &self,
            task_id: &str,
            _name: &str,
            _port: u16,
        ) -> Result<Option<String>, SandboxError> {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl SandboxStarter for UnreachableSandboxes {
        async fn start(
            &self,
            task_id: &str,
        ) -> Result<(), moatless_backend_core::sandbox::SandboxError> {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl SandboxStopper for UnreachableSandboxes {
        async fn stop(
            &self,
            task_id: &str,
            _reason: moatless_types::sandbox::SandboxStopReason,
        ) -> Result<(), moatless_backend_core::sandbox::SandboxError> {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl ClusterDiagnostics for UnreachableSandboxes {
        async fn list_sandbox_diagnostics(
            &self,
        ) -> Result<
            Vec<moatless_backend_core::sandbox::SandboxDiagnosticsEntry>,
            moatless_backend_core::sandbox::SandboxError,
        > {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl SandboxObserver for UnreachableSandboxes {
        async fn sandbox_status(
            &self,
            task_id: &str,
            _task_status: moatless_types::task::TaskStatus,
            _desired_state: moatless_types::sandbox::SandboxDesiredState,
        ) -> Result<ContractSandboxStatus, SandboxError> {
            Err(unreachable(task_id))
        }

        async fn live_status(
            &self,
            task_id: &str,
            _task_status: moatless_types::task::TaskStatus,
            _desired_state: moatless_types::sandbox::SandboxDesiredState,
        ) -> Result<LiveStatus, SandboxError> {
            Err(unreachable(task_id))
        }

        async fn server_logs(
            &self,
            task_id: &str,
            _server_name: &str,
            _options: LogOptions,
        ) -> Result<ServerLogStream, SandboxError> {
            Err(unreachable(task_id))
        }
    }

    struct TestPairingCodes(Option<PairingCode>);

    #[async_trait]
    impl PairingCodeDao for TestPairingCodes {
        async fn insert_pairing_code(&self, _code: &PairingCode) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn consume_pairing_code(
            &self,
            code_hash: &str,
            _now: DateTime<Utc>,
        ) -> Result<Option<PairingCode>, sqlx::Error> {
            Ok(self
                .0
                .as_ref()
                .filter(|code| code.code_hash == code_hash)
                .cloned())
        }

        async fn delete_finished_pairing_codes(
            &self,
            _cutoff: DateTime<Utc>,
        ) -> Result<u64, sqlx::Error> {
            Ok(0)
        }
    }

    struct TestApiKeyIssuer;

    #[async_trait]
    impl PairingApiKeyIssuer for TestApiKeyIssuer {
        async fn issue_api_key(
            &self,
            _user_id: &str,
            name: &str,
            scopes: Vec<String>,
            expires_at: DateTime<Utc>,
            _group_id: Option<&str>,
        ) -> Result<PairedDevice, PairingError> {
            assert_eq!(name, "T3 Code Mobile (iOS)");
            Ok(PairedDevice {
                key_id: "key-1".into(),
                token: "mvk_test-token".into(),
                expires_at,
                scopes,
            })
        }
    }

    fn test_pairing(code: Option<PairingCode>) -> DevicePairing {
        DevicePairing::new(Arc::new(TestPairingCodes(code)), Arc::new(TestApiKeyIssuer))
    }

    fn pairing_code(code: &str) -> PairingCode {
        PairingCode {
            id: "pairing-1".into(),
            code_hash: hash_key(code),
            user_id: Uuid::nil().to_string(),
            group_id: None,
            label: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        }
    }

    /// The full catalog with everything installed. The provider list's own
    /// shape is asserted in `providers.rs`; here it only has to be non-empty so
    /// the composer is not disabled.
    struct StubAgents;

    #[async_trait]
    impl ports::AgentCatalogSource for StubAgents {
        async fn list(
            &self,
            _viewer: &ScopeContext,
        ) -> Vec<moatless_api_schemas::agent::AgentDescriptor> {
            crate::agent::catalog::catalog()
                .iter()
                .map(|entry| moatless_api_schemas::agent::AgentDescriptor {
                    id: entry.id.to_string(),
                    label: entry.label.to_string(),
                    installed: true,
                    default_model: entry.default_model.to_string(),
                    models: entry
                        .models
                        .iter()
                        .map(|m| moatless_api_schemas::agent::AgentModel {
                            id: m.id.to_string(),
                            label: m.label.to_string(),
                        })
                        .collect(),
                    missing_credential_hint: None,
                })
                .collect()
        }
    }

    /// No test in this file dispatches a project-root `vcs.listRefs`, so the
    /// git-host branch listing is never reached here; it is asserted in
    /// `review.rs` against a mock workspace DAO and branch source.
    struct UnreachableBranches;

    #[async_trait]
    impl ports::RepositoryBranchSource for UnreachableBranches {
        async fn list_branches(
            &self,
            _viewer: &ScopeContext,
            _repository_id: &str,
            _workspace_id: Option<&str>,
        ) -> Result<moatless_api_schemas::repository::BranchListResponse, ports::BranchListError>
        {
            unreachable!("no test in this file lists a project's branches")
        }
    }

    /// A Task in these envelope tests is never bound to a pull request; what a
    /// bound one projects to is asserted in `review.rs`.
    struct NoBoundPullRequests;

    #[async_trait]
    impl ports::BoundPullRequestSource for NoBoundPullRequests {
        async fn bound_pull_request(&self, _task_id: &str) -> Option<ports::BoundPullRequest> {
            None
        }
        async fn resolve_pull_request(
            &self,
            _viewer: &ScopeContext,
            _repository_id: &str,
            _reference: &str,
        ) -> Result<ports::ResolvedPullRequest, ports::PullRequestError> {
            unreachable!("no test in this file resolves a pull request")
        }
        async fn open_pull_request_for_head(
            &self,
            _viewer: &ScopeContext,
            _repository_id: &str,
            _head: &str,
        ) -> Option<ports::ResolvedPullRequest> {
            None
        }
    }

    /// The envelope tests never sign in with GitHub; what an authenticated badge
    /// projects to is asserted in `git_actions.rs`.
    struct NoGithubIdentity;

    #[async_trait]
    impl ports::SourceControlAuthSource for NoGithubIdentity {
        async fn github_identity(&self, _viewer: &ScopeContext) -> Option<ports::GithubIdentity> {
            None
        }
    }

    struct NoChildTasks;

    #[async_trait]
    impl ports::TaskRelationSource for NoChildTasks {
        async fn children(
            &self,
            _parent_task_id: &str,
        ) -> Result<Vec<ports::ChildTask>, moatless_types::store::StoreError> {
            Ok(Vec::new())
        }
    }

    /// What reached the attachment store, so the mint, the upload route and
    /// `attachments.delete` can be asserted without a database or an S3 bucket
    /// behind them.
    #[derive(Default)]
    struct RecordingAttachments {
        /// Each stored attachment as `(id, filename, mime type, byte count)`.
        stored: std::sync::Mutex<Vec<(String, String, String, usize)>>,
        deleted: std::sync::Mutex<Vec<String>>,
        refuse: Option<ports::AttachmentStoreError>,
    }

    impl RecordingAttachments {
        fn refusal(&self) -> Result<(), ports::AttachmentStoreError> {
            match &self.refuse {
                None => Ok(()),
                Some(ports::AttachmentStoreError::Forbidden) => {
                    Err(ports::AttachmentStoreError::Forbidden)
                }
                Some(ports::AttachmentStoreError::Failed(message)) => {
                    Err(ports::AttachmentStoreError::Failed(message.clone()))
                }
            }
        }
    }

    #[async_trait]
    impl ports::AttachmentStore for RecordingAttachments {
        async fn store(
            &self,
            _viewer: &ScopeContext,
            attachment_id: &str,
            declared: ports::DeclaredAttachment,
            bytes: Vec<u8>,
        ) -> Result<(), ports::AttachmentStoreError> {
            #[allow(clippy::unwrap_used)]
            self.stored.lock().unwrap().push((
                attachment_id.to_string(),
                declared.filename,
                declared.mime_type,
                bytes.len(),
            ));
            self.refusal()
        }

        async fn delete(
            &self,
            _viewer: &ScopeContext,
            attachment_id: &str,
        ) -> Result<(), ports::AttachmentStoreError> {
            #[allow(clippy::unwrap_used)]
            self.deleted.lock().unwrap().push(attachment_id.to_string());
            self.refusal()
        }
    }

    /// Nothing in this file dispatches a command that reaches a write; the
    /// command table itself is asserted in `commands.rs` against a recording
    /// sink.
    struct UnreachableCommands;

    #[async_trait]
    impl ports::ThreadCommandSink for UnreachableCommands {
        async fn send_turn(
            &self,
            _viewer: &ScopeContext,
            _turn: ports::ThreadTurn,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file sends a turn")
        }
        async fn create_thread(
            &self,
            _viewer: &ScopeContext,
            _thread: ports::NewThread,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file creates a thread")
        }
        async fn update_meta(
            &self,
            _viewer: &ScopeContext,
            _meta: ports::ThreadMeta,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file updates metadata")
        }
        async fn stop(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file stops a thread")
        }
        async fn archive(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file archives a thread")
        }
        async fn unarchive(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file unarchives a thread")
        }
        async fn settle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file settles a thread")
        }
        async fn unsettle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file unsettles a thread")
        }
        async fn snooze(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _snoozed_until: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file snoozes a thread")
        }
        async fn unsnooze(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file unsnoozes a thread")
        }
        async fn answer_question(
            &self,
            _viewer: &ScopeContext,
            _answer: ports::QuestionAnswer,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file answers a question")
        }
        async fn fork(
            &self,
            _viewer: &ScopeContext,
            _fork: ports::ForkThread,
        ) -> Result<(), ports::CommandRefusal> {
            unreachable!("no test in this file forks a thread")
        }
    }

    /// These tests are about the envelope encoding, so the listing behind it
    /// is a stub. What a viewer actually sees is asserted against a real
    /// database in `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
    fn state() -> UiRpcState {
        state_with_attachments(Arc::new(RecordingAttachments::default()))
    }

    fn state_with_attachments(attachments: Arc<dyn ports::AttachmentStore>) -> UiRpcState {
        let mut workspaces = MockT3WorkspaceDao::new();
        workspaces
            .expect_list_readable_including()
            .returning(|_, _| Ok(Vec::new()));
        let mut conversations = MockConversationDao::new();
        conversations
            .expect_list_followed()
            .returning(|_| Ok(Vec::new()));
        conversations
            .expect_list_followed_by_ids()
            .returning(|_, _| Ok(Vec::new()));
        conversations
            .expect_readable_by_id()
            .returning(|_, _| Ok(None));
        conversations.expect_listing_head().returning(|| {
            Ok(UiRpcListingHead {
                identity: stub_identity(),
                position: 0,
            })
        });
        conversations
            .expect_listing_changes_after()
            .returning(|_, _| Ok(Vec::new()));
        // The stub holds no conversations, so every one asked for is one that
        // is gone: nothing has ever changed and nothing is named. What a
        // conversation that exists projects to is asserted in `projection.rs`.
        conversations
            .expect_conversation_head()
            .returning(|_| Ok(ConversationHead { position: 0 }));
        conversations
            .expect_conversation_changes()
            .returning(|_, _, _| Ok(Vec::new()));
        conversations
            .expect_readable_by_ids()
            .returning(|_, _| Ok(Vec::new()));
        let conversations: Arc<dyn ConversationDao> = Arc::new(conversations);
        UiRpcState {
            rpc: Arc::new(UiRpcConfig {
                environment_id: environment_id(&stub_identity()),
                allowed_origins: vec![AllowedOrigin::parse("http://localhost:5733")],
                otlp_traces_enabled: false,
                attachment_uploads: true,
            }),
            environment: Arc::new(stub_identity()),
            session_cookie_name: Arc::from("moatless_session"),
            frontend_url: Arc::from("http://localhost:3000"),
            session_jwt_secret: Some(Arc::from("test-secret")),
            pairing: Arc::new(test_pairing(None)),
            workspaces: Arc::new(workspaces),
            conversations: conversations.clone(),
            sandboxes: Arc::new(UnreachableSandboxes),
            servers: ServersState {
                observer: Arc::new(UnreachableSandboxes),
                tasks: Arc::new(MockT3TaskDao::new()),
                intent: Arc::new(UnreachableSandboxes),
            },
            scripts: ScriptsState {
                workspaces: Arc::new(MockT3WorkspaceDao::new()),
                ports: Arc::new(UnreachableSandboxes),
            },
            sandbox: SandboxState {
                observer: Arc::new(UnreachableSandboxes),
                tasks: Arc::new(MockT3TaskDao::new()),
                intent: Arc::new(UnreachableSandboxes),
                starter: Arc::new(UnreachableSandboxes),
                stopper: Arc::new(UnreachableSandboxes),
                diagnostics: Arc::new(UnreachableSandboxes),
            },
            subtasks: SubtasksState {
                relations: Arc::new(NoChildTasks),
                conversations,
            },
            preview: PreviewState {
                tabs: Arc::new(preview_tabs::InMemoryPreviewTabDao::new()),
                tasks: Arc::new(MockT3TaskDao::new()),
            },
            agents: Arc::new(StubAgents),
            branches: Arc::new(UnreachableBranches),
            pull_requests: Arc::new(NoBoundPullRequests),
            source_control_auth: Arc::new(NoGithubIdentity),
            commands: Arc::new(UnreachableCommands),
            attachments,
            model_defaults: Arc::new(AgentModelDefaults::default()),
            terminal: terminal::TerminalHub::default(),
        }
    }

    /// A session whose pushed frames go nowhere. These tests drive the request
    /// side; what a subscription's poller produces is asserted against a real
    /// database in `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
    fn session() -> ProtocolSession {
        ProtocolSession::new(mpsc::channel(OUTBOUND_FRAMES).0)
    }

    fn viewer() -> UiRpcAuthIdentity {
        UiRpcAuthIdentity {
            user_id: Uuid::nil(),
            login: "viewer".into(),
            role: UserRole::User,
            provider: "test".into(),
            group_id: None,
        }
    }

    fn request(id: &str, tag: &str, payload: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": id,
            "tag": tag,
            "payload": payload,
            "headers": []
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn a_request_id_returns_in_the_json_type_it_arrived_as() {
        // The client resolves a response by looking `requestId` up in a map
        // keyed by the value it sent, and coerces nothing. Answering `21` with
        // `"21"` matches no caller, so the request never completes and the
        // connection attempt times out with every frame looking correct.
        for id in [json!(21), json!("21")] {
            let frame = serde_json::to_vec(&json!({
                "_tag": "Request",
                "id": id,
                "tag": "server.getConfig",
                "payload": {},
                "headers": []
            }))
            .unwrap();
            let mut session = session();
            let output = session.handle_json(&frame, &state(), &viewer()).await;
            assert_eq!(output.frames[0]["_tag"], "Exit");
            assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
            assert_eq!(
                output.frames[0]["requestId"], id,
                "a {id} request id must come back as {id}, not re-typed"
            );
        }
    }

    #[tokio::test]
    async fn a_numeric_stream_id_returns_numeric_on_every_frame() {
        let frame = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": 4,
            "tag": "subscribeServerConfig",
            "payload": {},
            "headers": []
        }))
        .unwrap();
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;
        assert_eq!(output.frames[0]["_tag"], "Chunk");
        assert_eq!(
            output.frames[0]["requestId"],
            json!(4),
            "a stream's chunks correlate by the same id its request carried"
        );

        let interrupt =
            serde_json::to_vec(&json!({ "_tag": "Interrupt", "requestId": 4 })).unwrap();
        let output = session.handle_json(&interrupt, &state(), &viewer()).await;
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["requestId"], json!(4));
    }

    /// A fleet with one Sandbox in it, for the dispatch test below. Real
    /// mapping from pod list and metrics into this shape is unit-tested
    /// against fixtures in `sandbox::k8s::pod_reader`; this only checks that
    /// the RPC arm serializes what the seam hands it, unchanged.
    struct FakeFleet;

    #[async_trait]
    impl ClusterDiagnostics for FakeFleet {
        async fn list_sandbox_diagnostics(
            &self,
        ) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError> {
            Ok(vec![SandboxDiagnosticsEntry {
                task_id: "task-1".into(),
                title: None,
                sandbox_status: SandboxStatus::Ready,
                node_name: Some("node-a".into()),
                containers: vec![SandboxDiagnosticsContainer {
                    name: "sandbox".into(),
                    state: "running".into(),
                    restart_count: 0,
                    cpu_millicores: Some(100),
                    memory_mb: Some(256),
                }],
                total_cpu_millicores: Some(100),
                total_memory_mb: Some(256),
            }])
        }
    }

    #[tokio::test]
    async fn a_sandbox_list_diagnostics_request_returns_the_mapped_fleet() {
        let mut state = state();
        state.sandbox.diagnostics = Arc::new(FakeFleet);
        let frame = request("1", "sandbox.listDiagnostics", json!({}));
        let mut session = session();
        let output = session.handle_json(&frame, &state, &viewer()).await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        let sandboxes = &output.frames[0]["exit"]["value"]["sandboxes"];
        assert_eq!(sandboxes[0]["taskId"], "task-1");
        assert_eq!(sandboxes[0]["nodeName"], "node-a");
        assert_eq!(sandboxes[0]["containers"][0]["name"], "sandbox");
        assert_eq!(sandboxes[0]["containers"][0]["cpuMillicores"], 100);
        assert_eq!(sandboxes[0]["totalMemoryMb"], 256);
    }

    /// An observer that reports one fixed status, for the subscribe test below.
    struct FixedStatus(SandboxStatus);

    #[async_trait]
    impl SandboxIntentReader for FixedStatus {
        async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
            Ok(SandboxDesiredState::Running)
        }
    }

    #[async_trait]
    impl SandboxObserver for FixedStatus {
        async fn sandbox_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<ContractSandboxStatus, SandboxError> {
            unreachable!("the lifecycle read uses live_status")
        }

        async fn live_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<LiveStatus, SandboxError> {
            Ok(LiveStatus {
                sandbox_status: self.0,
                agent_status: None,
                sandbox_error: None,
            })
        }

        async fn server_logs(
            &self,
            _task_id: &str,
            _server_name: &str,
            _options: LogOptions,
        ) -> Result<ServerLogStream, SandboxError> {
            unreachable!("the lifecycle read reads no logs")
        }
    }

    fn state_with_sandbox_status(status: SandboxStatus) -> UiRpcState {
        let mut state = state();
        let mut tasks = MockT3TaskDao::new();
        tasks.expect_find_by_id().returning(|_, _| {
            Ok(Some(TaskView {
                status: TaskStatus::Open,
            }))
        });
        state.sandbox.tasks = Arc::new(tasks);
        state.sandbox.observer = Arc::new(FixedStatus(status));
        state.sandbox.intent = Arc::new(FixedStatus(status));
        state
    }

    /// The subscription seeds before its first poll interval elapses.
    ///
    /// Without the seed the indicator would render nothing for the first
    /// interval on every mount — which is the stale-status bug the push channel
    /// exists to end, just moved to the start of the stream.
    #[tokio::test]
    async fn subscribing_to_a_sandbox_status_answers_before_the_first_poll() {
        let frame = request("1", "sandbox.subscribeStatus", json!({ "threadId": "task-1" }));
        let mut session = session();
        let output = session
            .handle_json(&frame, &state_with_sandbox_status(SandboxStatus::Ready), &viewer())
            .await;

        assert_eq!(output.frames[0]["_tag"], "Chunk");
        assert_eq!(output.frames[0]["values"][0]["sandboxStatus"], "ready");
    }

    /// A subscription is per-thread, so the thread is not optional. Left to the
    /// poller this would be a stream that authorizes nothing and never emits.
    #[tokio::test]
    async fn subscribing_to_a_sandbox_status_without_a_thread_is_refused() {
        let frame = request("1", "sandbox.subscribeStatus", json!({}));
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
        assert!(
            output.frames[0]["exit"]["cause"][0]["defect"]
                .as_str()
                .unwrap_or_default()
                .contains("threadId"),
            "the refusal should name what is missing, got {output:?}"
        );
    }

    /// The capability is how a client knows it may stop polling `sandbox.status`.
    /// A server that serves the subscription but stays quiet about it leaves
    /// every client on the poll.
    #[tokio::test]
    async fn the_descriptor_advertises_the_sandbox_status_push() {
        let frame = request("1", "server.getConfig", json!({}));
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;

        assert_eq!(
            output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxStatusPush"],
            json!(true)
        );
    }

    /// `agentStatus` is `optionalKey` and absent whenever the sandbox is not
    /// up, so its absence from one answer says nothing about the server. The
    /// capability is the only thing that distinguishes a server that reports it
    /// from one that never will.
    #[tokio::test]
    async fn the_descriptor_advertises_the_sandbox_agent_status() {
        let frame = request("1", "server.getConfig", json!({}));
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;

        assert_eq!(
            output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxAgentStatus"],
            json!(true)
        );
    }

    /// The two keys the composer reads before it decides how to send a file.
    /// Reporting `attachmentUploads` moves it off the inline data-URL path for
    /// images too, so a server that reports one without serving the other
    /// loses the person their attachment.
    #[tokio::test]
    async fn the_descriptor_advertises_both_attachment_capabilities_together() {
        let frame = request("1", "server.getConfig", json!({}));
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;
        let capabilities = &output.frames[0]["exit"]["value"]["environment"]["capabilities"];

        assert_eq!(capabilities["attachmentUploads"], json!(true));
        assert_eq!(
            capabilities["fileAttachments"],
            json!({ "maxUploadBytes": 50 * 1024 * 1024 })
        );
    }

    /// Both keys are `optionalKey` in the contract, so a deployment that cannot
    /// sign an upload URL has to omit them rather than answer `false` or
    /// `null`: the client decodes the whole descriptor before it can connect.
    #[test]
    fn a_deployment_that_cannot_sign_an_upload_url_omits_both_keys() {
        let config = UiRpcConfig {
            environment_id: "moatless".into(),
            allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
            otlp_traces_enabled: false,
            attachment_uploads: false,
        };
        let capabilities = &descriptor(&config)["capabilities"];

        assert!(capabilities.get("attachmentUploads").is_none());
        assert!(capabilities.get("fileAttachments").is_none());
    }

    /// What the composer gets back before it moves any bytes: the id it will
    /// name on the turn, and the URL it will `POST` to.
    #[tokio::test]
    async fn create_upload_url_answers_an_id_and_a_url_under_the_upload_prefix() {
        let frame = request(
            "1",
            "attachments.createUploadUrl",
            json!({ "type": "file", "name": "report.pdf", "mimeType": "application/pdf", "sizeBytes": 2048 }),
        );
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;
        let value = &output.frames[0]["exit"]["value"];

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert!(value["attachmentId"].as_str().is_some_and(|id| !id.is_empty()));
        assert!(
            value["relativeUrl"]
                .as_str()
                .is_some_and(|url| url.starts_with("/api/attachments/")),
            "{value}"
        );
        assert!(value["expiresAt"].as_i64().is_some_and(|at| at > 0));
    }

    /// A payload outside the union never becomes an upload URL. The client's
    /// own schema refuses these before they are sent, so this is a malformed
    /// request rather than one of the method's declared outcomes.
    #[tokio::test]
    async fn create_upload_url_refuses_a_payload_outside_the_union() {
        let frame = request(
            "1",
            "attachments.createUploadUrl",
            json!({ "type": "image", "name": "a.tiff", "mimeType": "image/tiff", "sizeBytes": 10 }),
        );
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;

        assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
    }

    /// The composer deletes an attachment it abandoned — a cancel, a retry, a
    /// discarded draft — without knowing whether the upload ever landed, so
    /// this succeeds either way and the id is what has to reach the store.
    #[tokio::test]
    async fn delete_reaches_the_store_with_the_id_it_was_given() {
        let attachments = Arc::new(RecordingAttachments::default());
        let frame = request(
            "1",
            "attachments.delete",
            json!({ "attachmentId": "6b1f7b2e-0000-4000-8000-000000000001" }),
        );
        let mut session = session();
        let output = session
            .handle_json(&frame, &state_with_attachments(attachments.clone()), &viewer())
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        #[allow(clippy::unwrap_used)]
        let deleted = attachments.deleted.lock().unwrap().clone();
        assert_eq!(deleted, vec!["6b1f7b2e-0000-4000-8000-000000000001"]);
    }

    /// Somebody else's attachment is, from here, one that is not there:
    /// answering differently would make this a way to ask whether an id exists.
    #[tokio::test]
    async fn delete_of_an_attachment_this_viewer_may_not_touch_still_succeeds() {
        let attachments = Arc::new(RecordingAttachments {
            refuse: Some(ports::AttachmentStoreError::Forbidden),
            ..RecordingAttachments::default()
        });
        let frame = request("1", "attachments.delete", json!({ "attachmentId": "abc" }));
        let mut session = session();
        let output = session
            .handle_json(&frame, &state_with_attachments(attachments), &viewer())
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    }

    /// An id that could leave the route it becomes a segment of never reaches
    /// the store.
    #[tokio::test]
    async fn delete_of_something_that_is_not_an_id_never_reaches_the_store() {
        let attachments = Arc::new(RecordingAttachments::default());
        let frame = request(
            "1",
            "attachments.delete",
            json!({ "attachmentId": "../secrets" }),
        );
        let mut session = session();
        let output = session
            .handle_json(&frame, &state_with_attachments(attachments.clone()), &viewer())
            .await;

        assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
        #[allow(clippy::unwrap_used)]
        let deleted = attachments.deleted.lock().unwrap().len();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn a_numeric_id_is_accepted_on_every_frame_that_carries_one() {
        for frame in [
            json!({ "_tag": "Ack", "requestId": 7 }),
            json!({ "_tag": "Interrupt", "requestId": 7 }),
        ] {
            let tag = frame["_tag"].as_str().unwrap().to_string();
            let mut session = session();
            let output = session
                .handle_json(&serde_json::to_vec(&frame).unwrap(), &state(), &viewer())
                .await;
            let defected = output.frames.iter().any(|frame| frame["_tag"] == "Defect");
            assert!(
                !defected,
                "{tag} with a numeric id should parse, got {output:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_fractional_request_id_is_refused() {
        let frame = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": 21.5,
            "tag": "server.getConfig",
            "payload": {},
            "headers": []
        }))
        .unwrap();
        let mut session = session();
        let output = session.handle_json(&frame, &state(), &viewer()).await;
        assert_eq!(output.frames[0]["_tag"], "Defect");
    }

    #[tokio::test]
    async fn ping_returns_effect_rpc_pong() {
        let mut session = session();
        let output = session
            .handle_json(br#"{"_tag":"Ping"}"#, &state(), &viewer())
            .await;
        assert_eq!(output.frames, vec![json!({ "_tag": "Pong" })]);
    }

    #[tokio::test]
    async fn get_config_returns_minimal_decodable_wire_shape() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("1", "server.getConfig", json!({})),
                &state(),
                &viewer(),
            )
            .await;
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(
            output.frames[0]["exit"]["value"]["environment"]["environmentId"],
            environment_id(&stub_identity()),
            "the reported environment id is the configured name and the \
             database's digest"
        );
        assert_eq!(
            output.frames[0]["exit"]["value"]["auth"]["sessionCookieName"],
            "moatless_session"
        );
        assert_eq!(
            output.frames[0]["exit"]["value"]["auth"]["policy"],
            "remote-reachable"
        );
        assert_eq!(
            output.frames[0]["exit"]["value"]["auth"]["bootstrapMethods"],
            json!(["one-time-token"]),
            "a client reads here that it may pair with a code"
        );
        assert_eq!(
            output.frames[0]["exit"]["value"]["auth"]["sessionMethods"],
            json!(["browser-session-cookie", "bearer-access-token"])
        );
        assert_eq!(
            output.frames[0]["exit"]["value"]["threadResumeCompletionMarker"], true,
            "do not advertise this unless subscribeThread can emit synchronized"
        );
    }

    /// The client polls this for every conversation it holds, most of which
    /// have no Sandbox running to ask.
    #[tokio::test]
    async fn list_refs_answers_a_conversation_with_no_sandbox_rather_than_failing() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "98",
                    "vcs.listRefs",
                    json!({
                        "cwd": "/moatless/tasks/task-a",
                        "query": "main",
                        "limit": 10
                    }),
                ),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(
            output.frames[0]["exit"]["value"],
            json!({
                "refs": [],
                "isRepo": false,
                "hasPrimaryRemote": false,
                "nextCursor": null,
                "totalCount": 0
            })
        );
    }

    /// Picking a branch for a draft thread is answered as done, echoing the
    /// ref, so the client records it and sends it when the Task is created —
    /// there is no checkout to switch yet.
    #[tokio::test]
    async fn switching_a_draft_threads_branch_echoes_the_ref() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "97",
                    "vcs.switchRef",
                    json!({ "cwd": "/moatless/workspaces/ws-a", "refName": "feature" }),
                ),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(
            output.frames[0]["exit"]["value"],
            json!({ "refName": "feature" })
        );
    }

    /// A thread that already has a Sandbox cannot change branch — that is fixed
    /// at creation — so a switch addressed to a worktree is refused rather than
    /// silently claimed as done.
    #[tokio::test]
    async fn switching_an_existing_threads_branch_is_unsupported() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "96",
                    "vcs.switchRef",
                    json!({ "cwd": "/moatless/tasks/task-a", "refName": "feature" }),
                ),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
        let cause = &output.frames[0]["exit"]["cause"][0];
        assert_eq!(cause["error"]["_tag"], "UnsupportedMethodError");
    }

    /// The diff panel is a surface the person opened on purpose, so a
    /// conversation it cannot read says why — in the error type the method
    /// declares, which is what the panel renders instead of a defect.
    #[tokio::test]
    async fn a_refused_diff_preview_fails_with_the_method_s_own_error() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "99",
                    "review.getDiffPreview",
                    json!({ "cwd": "/moatless/tasks/task-a", "ignoreWhitespace": true }),
                ),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
        let cause = &output.frames[0]["exit"]["cause"][0]["error"];
        assert_eq!(cause["_tag"], "GitCommandError");
        assert_eq!(cause["operation"], "review.getDiffPreview");
        assert_eq!(
            cause["detail"], "This conversation's workspace is not available.",
            "a cwd the viewer's listing does not hold must not report whether the conversation exists"
        );
    }

    /// Opened for every conversation the client shows, including the ones this
    /// surface has no path to. A stream that says nothing is the answer there;
    /// a failure would be one the client keeps.
    #[tokio::test]
    async fn a_vcs_status_subscription_without_an_addressable_cwd_simply_stays_quiet() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("100", "subscribeVcsStatus", json!({ "cwd": "   " })),
                &state(),
                &viewer(),
            )
            .await;

        assert!(output.frames.is_empty(), "no chunk, and no failure either");
    }

    /// The client calls this one on its own schedule and when a worktree is
    /// cleaned up, so an unanswerable `cwd` has to come back as the method's
    /// error rather than the defect an unimplemented method produces.
    #[tokio::test]
    async fn a_status_refresh_answers_rather_than_dying() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "101",
                    "vcs.refreshStatus",
                    json!({ "cwd": "/moatless/tasks/task-a" }),
                ),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
        let cause = &output.frames[0]["exit"]["cause"][0];
        assert_eq!(cause["_tag"], "Fail", "a typed refusal, not a Die defect");
        assert_eq!(cause["error"]["_tag"], "GitCommandError");
        assert_eq!(cause["error"]["operation"], "vcs.refreshStatus");
    }

    /// The composer is disabled while the provider list is empty
    /// (`ChatView.logic.ts` has nothing to resolve a model against), so this is
    /// the frame that decides whether anybody can type at all.
    #[tokio::test]
    async fn get_config_publishes_one_provider_per_agent_kind() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("1", "server.getConfig", json!({})),
                &state(),
                &viewer(),
            )
            .await;

        let providers = output.frames[0]["exit"]["value"]["providers"]
            .as_array()
            .expect("the config carries a provider list");
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider["instanceId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["claudeAgent", "codex"]
        );
        for provider in providers {
            assert!(
                !provider["models"].as_array().unwrap().is_empty(),
                "{} published no models, so its composer resolves nothing",
                provider["instanceId"]
            );
        }
    }

    /// T3 calls `refreshProviders` when a person opens the picker, and merges
    /// the answer into the same state the config populated. Two lists that
    /// disagree would move a thread's provider under it.
    #[tokio::test]
    async fn refreshing_providers_returns_what_the_config_already_published() {
        let mut session = session();
        let state = state();
        let config = session
            .handle_json(
                &request("1", "server.getConfig", json!({})),
                &state,
                &viewer(),
            )
            .await;
        let refreshed = session
            .handle_json(
                &request("2", "server.refreshProviders", json!({})),
                &state,
                &viewer(),
            )
            .await;

        let published = &config.frames[0]["exit"]["value"]["providers"];
        let refreshed = &refreshed.frames[0]["exit"]["value"]["providers"];
        // `checkedAt` is stamped per call and is the one field allowed to
        // differ; everything a client routes on has to match.
        let strip = |list: &Value| -> Vec<Value> {
            list.as_array()
                .unwrap()
                .iter()
                .map(|provider| {
                    let mut provider = provider.clone();
                    provider.as_object_mut().unwrap().remove("checkedAt");
                    provider
                })
                .collect()
        };
        assert_eq!(strip(published), strip(refreshed));
    }

    #[tokio::test]
    async fn shell_subscription_chunks_snapshot_and_marker_then_waits_for_ack() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "2",
                    "orchestration.subscribeShell",
                    json!({ "requestCompletionMarker": true }),
                ),
                &state(),
                &viewer(),
            )
            .await;
        assert_eq!(output.frames[0]["_tag"], "Chunk");
        assert_eq!(output.frames[0]["values"][0]["kind"], "snapshot");
        assert_eq!(output.frames[0]["values"][1]["kind"], "synchronized");
        assert!(session.streams["2"].awaiting_ack);

        let ack = session
            .handle_json(br#"{"_tag":"Ack","requestId":"2"}"#, &state(), &viewer())
            .await;
        assert!(ack.frames.is_empty());
        assert!(!session.streams["2"].awaiting_ack);
    }

    #[test]
    fn thread_initial_values_put_synchronization_after_initial_projection() {
        let snapshot = json!({
            "kind": "snapshot",
            "snapshot": {
                "snapshotSequence": 0,
                "thread": {
                    "messages": [{ "text": "Fix the build" }],
                    "activities": [{ "summary": "Thinking" }]
                }
            }
        });
        let values = thread_initial_values(vec![snapshot.clone()], true);
        assert_eq!(values, vec![snapshot, json!({ "kind": "synchronized" })]);

        let caught_up = thread_initial_values(Vec::new(), true);
        assert_eq!(caught_up, vec![json!({ "kind": "synchronized" })]);

        assert_eq!(
            thread_initial_values(vec![json!({ "kind": "snapshot" })], false),
            vec![json!({ "kind": "snapshot" })],
            "the marker is emitted only when T3 requested it"
        );
    }

    /// A client that resumes is not sent the listing again, and the completion
    /// marker still arrives — so it stops rendering itself as synchronizing.
    ///
    /// The stub's position is 0 and its change feed is empty, which is a client
    /// that is up to date: the correct answer is the marker and nothing else.
    /// Before this ticket every subscription began with a snapshot, so the
    /// first assertion is what changed.
    #[tokio::test]
    async fn a_resuming_subscription_chunks_no_snapshot() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "5",
                    "orchestration.subscribeShell",
                    json!({ "afterSequence": 0, "requestCompletionMarker": true }),
                ),
                &state(),
                &viewer(),
            )
            .await;
        assert_eq!(output.frames[0]["_tag"], "Chunk");
        let values = output.frames[0]["values"].as_array().unwrap();
        assert_eq!(
            values.len(),
            1,
            "a caught-up client should be sent the marker alone, got {values:#?}"
        );
        assert_eq!(values[0]["kind"], "synchronized");
    }

    /// `afterSequence` is a `NonNegativeInt` in the contract, so nothing a
    /// conforming client sends can be a negative number or a string. Reading
    /// one as absent rebuilds, which is a listing; reading it as a position
    /// would resume from a number nothing allocated.
    #[test]
    fn an_after_sequence_the_contract_refuses_is_read_as_absent() {
        assert_eq!(after_sequence(&json!({ "afterSequence": 7 })), Some(7));
        assert_eq!(after_sequence(&json!({ "afterSequence": 0 })), Some(0));
        assert_eq!(after_sequence(&json!({})), None);
        assert_eq!(after_sequence(&json!({ "afterSequence": null })), None);
        assert_eq!(after_sequence(&json!({ "afterSequence": -1 })), None);
        assert_eq!(after_sequence(&json!({ "afterSequence": "7" })), None);
        assert_eq!(after_sequence(&json!({ "afterSequence": 1.5 })), None);
    }

    /// A conversation this viewer cannot see ends the subscription rather than
    /// opening one that will never produce a frame.
    ///
    /// The client renders a conversation as still loading until a snapshot
    /// arrives, and a stream that opens and stays silent is indistinguishable
    /// from one that is slow — which is exactly what an unimplemented
    /// `subscribeThread` looked like from the browser.
    #[tokio::test]
    async fn a_thread_subscription_that_cannot_be_opened_fails_rather_than_hangs() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "6",
                    "orchestration.subscribeThread",
                    json!({ "threadId": "task-gone", "requestCompletionMarker": true }),
                ),
                &state(),
                &viewer(),
            )
            .await;
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
        assert!(!session.streams.contains_key("6"));
    }

    #[tokio::test]
    async fn a_thread_subscription_without_a_thread_is_refused() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("7", "orchestration.subscribeThread", json!({})),
                &state(),
                &viewer(),
            )
            .await;
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    }

    /// The client decodes a method's failure against that method's declared
    /// error type, so a tagged error is one of its ordinary outcomes and a
    /// string defect is not. Answering "no favicon" with a defect leaves the
    /// same blank icon behind a stack trace.
    #[tokio::test]
    async fn an_asset_this_environment_has_no_file_for_fails_in_the_contracts_own_type() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "8",
                    "assets.createUrl",
                    json!({ "resource": { "_tag": "project-favicon", "cwd": "/moatless" } }),
                ),
                &state(),
                &viewer(),
            )
            .await;
        let cause = &output.frames[0]["exit"]["cause"][0];
        assert_eq!(cause["_tag"], "Fail");
        assert_eq!(cause["error"]["_tag"], "AssetWorkspaceAssetNotFoundError");
        assert_eq!(
            cause["error"]["resource"],
            json!({ "_tag": "project-favicon", "cwd": "/moatless" }),
            "the error carries back the resource the request named"
        );
    }

    /// The client reports focus on every change so a host can idle its own
    /// background work. Moatless runs its agents in Sandboxes no browser tab is
    /// attached to, so the report is accepted and dropped — refusing it would
    /// put a defect in the console on every tab switch.
    #[tokio::test]
    async fn a_client_activity_report_is_accepted() {
        let mut session = session();
        let output = session
            .handle_json(
                &request(
                    "9",
                    "server.reportClientActivity",
                    json!({ "state": "active" }),
                ),
                &state(),
                &viewer(),
            )
            .await;
        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(output.frames[0]["exit"]["value"], Value::Null);
    }

    /// The difference between a `Fail` and a `Die` is what the person sees: a
    /// declared error renders its own message, a defect renders as "unexpected
    /// server error" and nothing else. A method this server does not have has
    /// not failed, and saying so is not a defect.
    #[tokio::test]
    async fn a_method_this_server_does_not_have_says_so_in_a_type_the_client_reads() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("1", "orchestration.getFullThreadDiff", json!({})),
                &state(),
                &viewer(),
            )
            .await;

        let cause = &output.frames[0]["exit"]["cause"][0];
        assert_eq!(cause["_tag"], "Fail", "a typed absence, not a Die defect");
        assert_eq!(cause["error"]["_tag"], "UnsupportedMethodError");
        // Carried separately from the message so a client fanning out several
        // calls can tell which one came back absent without parsing text.
        assert_eq!(cause["error"]["method"], "orchestration.getFullThreadDiff");
    }

    /// Pairing advertises a scope list. Until this gate existed the list was
    /// decoration — every method ran whatever it said.
    #[tokio::test]
    async fn a_method_needing_a_scope_this_deployment_withholds_is_refused() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("1", "cloud.installRelayClient", json!({})),
                &state(),
                &viewer(),
            )
            .await;

        let cause = &output.frames[0]["exit"]["cause"][0];
        assert_eq!(cause["_tag"], "Fail");
        assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
        assert_eq!(cause["error"]["requiredScope"], "relay:write");
    }

    /// Terminal metadata is a served stream. It starts with an explicit
    /// empty snapshot so the client can distinguish "no terminals yet" from a
    /// connection that never produced a value.
    #[tokio::test]
    async fn a_terminal_metadata_subscription_starts_with_a_snapshot() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("7", "subscribeTerminalMetadata", json!({})),
                &state(),
                &viewer(),
            )
            .await;

        assert!(session.streams.contains_key("7"));
        assert_eq!(output.frames[0]["_tag"], "Chunk");
        assert_eq!(output.frames[0]["values"][0]["type"], "snapshot");
        assert_eq!(output.frames[0]["values"][0]["terminals"], json!([]));
    }

    /// The client asking is already showing the conversation. Answering `null`
    /// leaves it showing what the thread subscription gave it; failing would
    /// take a rendered transcript off the screen over a missing title.
    #[tokio::test]
    async fn a_listing_row_for_a_conversation_this_viewer_cannot_reach_is_null_not_an_error() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("1", "threads.getShell", json!({ "threadId": "task-1" })),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(output.frames[0]["exit"]["value"]["thread"], Value::Null);
    }

    /// Every client encodes against the contract, so a request without one is
    /// malformed rather than a conversation that is missing — and the two must
    /// not answer alike, or a client bug reads as an empty deployment.
    #[tokio::test]
    async fn a_listing_row_asked_for_without_a_thread_id_is_a_failure() {
        let mut session = session();
        let output = session
            .handle_json(
                &request("1", "threads.getShell", json!({})),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    }

    /// One character matches most of the corpus and says nothing, and it costs a
    /// scan of every Message body to say so.
    #[tokio::test]
    async fn a_thread_search_too_short_to_mean_anything_never_reaches_the_database() {
        let mut session = session();
        // `state()` sets no `search_conversations` expectation, so the mock
        // panics if this reaches it — which is the assertion.
        let output = session
            .handle_json(
                &request("1", "orchestration.searchThreads", json!({ "query": "a" })),
                &state(),
                &viewer(),
            )
            .await;

        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(output.frames[0]["exit"]["value"]["matches"], json!([]));
    }

    #[tokio::test]
    async fn interrupt_terminates_active_stream() {
        let mut session = session();
        // A subscription that stays open and says nothing — a thread with no
        // repository to report on.
        session
            .handle_json(
                &request("3", "subscribeVcsStatus", json!({})),
                &state(),
                &viewer(),
            )
            .await;
        assert!(session.streams.contains_key("3"));
        let output = session
            .handle_json(
                br#"{"_tag":"Interrupt","requestId":"3"}"#,
                &state(),
                &viewer(),
            )
            .await;
        assert!(!session.streams.contains_key("3"));
        assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Interrupt");
    }

    #[tokio::test]
    async fn a_json_array_is_one_effect_rpc_batch() {
        let mut session = session();
        let output = session.handle_json(
            br#"[{"_tag":"Ping"},{"_tag":"Request","id":"4","tag":"server.probe","payload":{},"headers":[]}]"#,
            &state(),
            &viewer(),
        ).await;
        assert_eq!(output.frames.len(), 2);
        assert_eq!(output.frames[0]["_tag"], "Pong");
        assert_eq!(output.frames[1]["_tag"], "Exit");
    }

    #[test]
    fn origin_check_is_exact_and_requires_origin() {
        let allowed = vec![AllowedOrigin::parse("http://localhost:5733")];
        let mut headers = HeaderMap::new();
        assert!(!origin_allowed(&headers, &allowed));
        headers.insert(header::ORIGIN, "http://localhost:5733".parse().unwrap());
        assert!(origin_allowed(&headers, &allowed));
        headers.insert(header::ORIGIN, "http://attacker.example".parse().unwrap());
        assert!(!origin_allowed(&headers, &allowed));
    }

    #[test]
    fn wildcard_entry_admits_hosts_under_its_domain() {
        let allowed = AllowedOrigin::parse("https://*.preview.moatless.xyz");
        assert_eq!(
            allowed,
            AllowedOrigin::Subdomain {
                prefix: "https://".into(),
                suffix: ".preview.moatless.xyz".into(),
            }
        );
        for origin in [
            "https://8d38cf39-4659-4a5e-92b1-49e8dfc9efb1--5733.preview.moatless.xyz",
            "https://anything.preview.moatless.xyz",
            "https://deeper.nested.preview.moatless.xyz",
        ] {
            assert!(allowed.matches(origin), "{origin} should be admitted");
        }
    }

    #[test]
    fn wildcard_entry_refuses_everything_outside_its_domain() {
        let allowed = AllowedOrigin::parse("https://*.preview.moatless.xyz");
        for origin in [
            // the domain itself, which the wildcard label does not stand for
            "https://preview.moatless.xyz",
            // an empty label
            "https://.preview.moatless.xyz",
            // the parent domain, and a lookalike registered elsewhere
            "https://moatless.xyz",
            "https://evil-preview.moatless.xyz",
            "https://preview.moatless.xyz.evil.example",
            // a scheme the entry does not name
            "http://a.preview.moatless.xyz",
            // a port makes it a different origin
            "https://a.preview.moatless.xyz:8443",
            // a path or userinfo smuggling the suffix past a bare ends_with
            "https://evil.example/x.preview.moatless.xyz",
            "https://evil.example@x.preview.moatless.xyz",
            "https://evil.example?.preview.moatless.xyz",
            "https://evil.example#.preview.moatless.xyz",
            "https://evil.example\\.preview.moatless.xyz",
        ] {
            assert!(!allowed.matches(origin), "{origin} should be refused");
        }
    }

    #[test]
    fn entry_without_a_wildcard_label_stays_exact() {
        assert_eq!(
            AllowedOrigin::parse("https://moatless.xyz"),
            AllowedOrigin::Exact("https://moatless.xyz".into())
        );
        // A `*` that is not the whole leading label is not a wildcard.
        let partial = AllowedOrigin::parse("https://*preview.moatless.xyz");
        assert!(matches!(partial, AllowedOrigin::Exact(_)));
        assert!(!partial.matches("https://a.preview.moatless.xyz"));
    }

    /// Both file methods answer in the shape their own contract declares, and
    /// not as a defect. A `Die` reaches the file panel as "Workspace query
    /// failed." with the reason stripped, so a refusal the person can act on —
    /// no sandbox — has to arrive as a `Fail` carrying the sentence to show.
    #[tokio::test]
    async fn a_refused_file_request_fails_with_the_method_s_own_error() {
        for (tag, payload, error_tag) in [
            (
                "projects.listEntries",
                json!({ "cwd": "/moatless/workspaces/ws-a" }),
                "ProjectListEntriesError",
            ),
            (
                "projects.readFile",
                json!({ "cwd": "/moatless/workspaces/ws-a", "relativePath": "README.md" }),
                "ProjectReadFileError",
            ),
        ] {
            let mut session = session();
            let output = session
                .handle_json(&request("7", tag, payload), &state(), &viewer())
                .await;
            let exit = &output.frames[0]["exit"];
            assert_eq!(exit["_tag"], "Failure", "{tag}");
            assert_eq!(exit["cause"][0]["_tag"], "Fail", "{tag}");
            assert_eq!(exit["cause"][0]["error"]["_tag"], error_tag);
            assert_eq!(
                exit["cause"][0]["error"]["message"],
                "This conversation has no sandbox, so it has no workspace yet.",
                "a project root is the cwd a thread with no worktree sends"
            );
        }
    }

    /// A `cwd` naming a conversation the viewer's listing does not contain is
    /// answered the same way as one naming nothing at all. The stub listing is
    /// empty, so this is every conversation.
    #[tokio::test]
    async fn a_cwd_outside_the_viewer_s_listing_is_indistinguishable_from_an_unknown_one() {
        for cwd in [
            "/moatless/tasks/task-a",
            "/somewhere/else",
            "/moatless/tasks/",
        ] {
            let mut session = session();
            let output = session
                .handle_json(
                    &request("8", "projects.listEntries", json!({ "cwd": cwd })),
                    &state(),
                    &viewer(),
                )
                .await;
            let exit = &output.frames[0]["exit"];
            assert_eq!(exit["cause"][0]["_tag"], "Fail", "{cwd}");
            assert_eq!(
                exit["cause"][0]["error"]["message"],
                "This conversation's workspace is not available.",
                "{cwd} must not report whether the conversation exists"
            );
        }
    }

    #[test]
    fn enabled_configuration_requires_an_origin() {
        let config = UiRpcConfig {
            environment_id: "moatless".into(),
            allowed_origins: vec![],
            otlp_traces_enabled: false,
            attachment_uploads: true,
        };
        assert!(config.validate().is_err());
    }

    /// A client refuses a server whose version differs from its own, so the
    /// descriptor answers with the contract constant.
    #[test]
    fn the_descriptor_reports_the_contract_version_the_code_implements() {
        let config = UiRpcConfig {
            environment_id: "moatless".into(),
            allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
            otlp_traces_enabled: false,
            attachment_uploads: true,
        };

        assert_eq!(
            descriptor(&config)["serverVersion"],
            json!(T3_CLIENT_CONTRACT_VERSION)
        );
        assert_ne!(
            T3_CLIENT_CONTRACT_VERSION,
            env!("CARGO_PKG_VERSION"),
            "the contract version tracks the T3 client, not this crate"
        );
    }

    #[test]
    fn a_ticket_holder_needs_no_origin_and_everyone_else_does() {
        assert!(!origin_checked(Some(Extension(
            UiRpcAuthMethod::WebSocketTicket
        ))));
        assert!(origin_checked(Some(Extension(
            UiRpcAuthMethod::SessionCookie
        ))));
        assert!(origin_checked(Some(Extension(UiRpcAuthMethod::ApiKey))));
        // An upgrade the middleware never saw is treated as the cookie case.
        assert!(origin_checked(None));
    }

    /// The route a browser frame fetches a workspace file from.
    mod workspace_assets {
        use super::*;
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        async fn text(response: Response) -> String {
            let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
                .await
                .unwrap();
            String::from_utf8(body.to_vec()).unwrap()
        }

        /// A minted URL is an address and not a capability, so the Task behind
        /// it is re-read through the same follow-and-access filter on every
        /// fetch. The stub listing holds no conversations, so this viewer
        /// follows none — and is told what someone asking for a Task that does
        /// not exist is told.
        #[tokio::test]
        async fn a_conversation_this_viewer_does_not_follow_serves_no_file() {
            let mut request = Request::builder()
                .uri("/api/assets/t-1/docs/report.html")
                .body(Body::empty())
                .unwrap();
            request.extensions_mut().insert(viewer());

            let response = router(state()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            // The route is mounted and refused it, rather than never matching.
            assert_eq!(
                text(response).await,
                crate::workspace::WorkspaceAccessError::Unaddressable.message()
            );
        }
    }

    /// The JSON read routes negotiate compression; the socket does not.
    mod http_compression {
        use super::*;
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        async fn shell(accept_encoding: Option<&str>) -> Response {
            let mut builder = Request::builder().uri("/api/orchestration/shell");
            if let Some(encoding) = accept_encoding {
                builder = builder.header("accept-encoding", encoding);
            }
            let mut request = builder.body(Body::empty()).unwrap();
            request.extensions_mut().insert(viewer());
            router(state()).oneshot(request).await.unwrap()
        }

        /// A client that offers gzip gets it: the layer is wired to this route,
        /// which is the ~5x saving on the thread bootstrap the client pulls over
        /// HTTP for exactly this reason.
        #[tokio::test]
        async fn a_client_that_offers_gzip_gets_a_gzipped_body() {
            let response = shell(Some("gzip")).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response
                    .headers()
                    .get(header::CONTENT_ENCODING)
                    .and_then(|value| value.to_str().ok()),
                Some("gzip")
            );
        }

        /// A client that offers nothing still gets a readable body: compression
        /// is negotiated, never forced on a client that cannot decode it.
        #[tokio::test]
        async fn a_client_that_offers_no_encoding_gets_an_uncompressed_body() {
            let response = shell(None).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
        }
    }

    /// The upload route, driven the way the composer drives it: mint a URL over
    /// the socket, then `POST` the raw file to it.
    mod attachment_uploads {
        use super::*;
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        /// Mint a URL for an attachment of `size` bytes and return the path to
        /// `POST` it to.
        async fn minted_url(state: &UiRpcState, size: usize) -> String {
            let frame = request(
                "1",
                "attachments.createUploadUrl",
                json!({
                    "type": "file",
                    "name": "report.pdf",
                    "mimeType": "application/pdf",
                    "sizeBytes": size
                }),
            );
            let mut session = session();
            let output = session.handle_json(&frame, state, &viewer()).await;
            output.frames[0]["exit"]["value"]["relativeUrl"]
                .as_str()
                .expect("a minted url")
                .to_string()
        }

        async fn upload(state: UiRpcState, url: &str, body: Vec<u8>) -> StatusCode {
            let request = Request::builder()
                .method("POST")
                .uri(url)
                .header("content-type", "application/pdf")
                .body(Body::from(body))
                .expect("a request");
            router(state)
                .oneshot(request)
                .await
                .expect("a response")
                .status()
        }

        /// The whole point of the route: bytes in, and the name and type the
        /// mint declared — not the ones the request carried — stored beside
        /// them under the id the client is already holding.
        #[tokio::test]
        async fn a_body_of_the_declared_size_is_stored_under_the_minted_id() {
            let attachments = Arc::new(RecordingAttachments::default());
            let state = state_with_attachments(attachments.clone());
            let url = minted_url(&state, 8).await;
            let id = url.rsplit('/').next().unwrap_or_default().to_string();

            assert_eq!(
                upload(state, &url, vec![7u8; 8]).await,
                StatusCode::NO_CONTENT
            );
            #[allow(clippy::unwrap_used)]
            let stored = attachments.stored.lock().unwrap().clone();
            assert_eq!(stored.len(), 1, "{stored:?}");
            assert_eq!(stored[0].1, "report.pdf");
            assert_eq!(stored[0].2, "application/pdf");
            assert_eq!(stored[0].3, 8);
            // The id is in the token, not in the path — the path *is* the
            // token — so this asserts the two are the same string.
            assert!(!id.is_empty());
            assert!(url.ends_with(&id));
        }

        /// The boundary, from both sides. A body one byte over its declaration
        /// is refused and stores nothing; a body of exactly the declared size
        /// is stored.
        #[tokio::test]
        async fn a_body_is_refused_at_the_first_byte_past_its_declaration() {
            let attachments = Arc::new(RecordingAttachments::default());
            let state = state_with_attachments(attachments.clone());
            let url = minted_url(&state, 8).await;

            assert_eq!(
                upload(state.clone(), &url, vec![7u8; 9]).await,
                StatusCode::BAD_REQUEST
            );
            #[allow(clippy::unwrap_used)]
            let stored = attachments.stored.lock().unwrap().len();
            assert_eq!(stored, 0, "an oversize body must not be stored");

            assert_eq!(
                upload(state, &url, vec![7u8; 8]).await,
                StatusCode::NO_CONTENT
            );
        }

        /// A body that ends short of its declaration is a truncated upload, and
        /// storing it would reach the agent as a corrupt file.
        #[tokio::test]
        async fn a_body_that_ends_short_of_its_declaration_is_refused() {
            let attachments = Arc::new(RecordingAttachments::default());
            let state = state_with_attachments(attachments.clone());
            let url = minted_url(&state, 8).await;

            assert_eq!(
                upload(state, &url, vec![7u8; 7]).await,
                StatusCode::BAD_REQUEST
            );
            assert_eq!(attachments.stored.lock().expect("no poison").len(), 0);
        }

        /// The token in the path is the credential. A forged one, an edited one
        /// and one from another deployment all write nothing — this route is
        /// exempt from the auth middleware precisely because it authenticates
        /// itself.
        #[tokio::test]
        async fn a_url_this_server_did_not_mint_stores_nothing() {
            let attachments = Arc::new(RecordingAttachments::default());
            let state = state_with_attachments(attachments.clone());
            let minted = minted_url(&state, 8).await;

            for url in [
                "/api/attachments/not-a-token".to_string(),
                format!("{minted}x"),
            ] {
                assert_eq!(
                    upload(state.clone(), &url, vec![7u8; 8]).await,
                    StatusCode::UNAUTHORIZED,
                    "{url}"
                );
            }
            assert_eq!(attachments.stored.lock().expect("no poison").len(), 0);
        }
    }

    /// The pairing exchange and the ticket route, over HTTP, as a client on
    /// another device drives them.
    mod pairing {
        use super::*;
        use crate::auth::verify_ws_ticket_jwt;
        use axum::body::Body;
        use axum::http::Request;
        use chrono::Utc;
        use tower::ServiceExt;

        const TEST_SECRET: &str = "test-secret";

        fn state_pairing(pairing: DevicePairing) -> UiRpcState {
            UiRpcState {
                pairing: Arc::new(pairing),
                ..state()
            }
        }

        fn exchange(body: &str) -> Request<Body> {
            Request::builder()
                .method("POST")
                .uri("/oauth/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap()
        }

        /// What the T3 mobile client sends, byte for byte.
        ///
        /// Captured off `bootstrapRemoteBearerSession` driven with the mobile
        /// app's own scopes and metadata, rather than written from the schema:
        /// the encoding is the part a hand-written fixture gets wrong, and it
        /// is `+` for a space and `%3A` for the colons in every URN.
        fn mobile_exchange(code: &str) -> Request<Body> {
            exchange(&format!(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
                 &subject_token={code}\
                 &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
                 &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token\
                 &scope=orchestration%3Aread+orchestration%3Aoperate+terminal%3Aoperate+review%3Awrite+relay%3Aread\
                 &client_label=T3+Code+Mobile&client_device_type=mobile&client_os=iOS"
            ))
        }

        async fn json(response: Response) -> Value {
            let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
                .await
                .unwrap();
            serde_json::from_slice(&body).unwrap()
        }

        #[tokio::test]
        async fn a_typed_code_becomes_the_token_the_device_keeps() {
            let pairing = test_pairing(Some(pairing_code("ABCDEFGHJKLM")));

            let response = router(state_pairing(pairing))
                .oneshot(mobile_exchange("ABCDEFGHJKLM"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);

            let body = json(response).await;
            assert_eq!(body["token_type"], "Bearer");
            assert_eq!(
                body["issued_token_type"],
                "urn:ietf:params:oauth:token-type:access_token"
            );
            assert!(body["access_token"].as_str().unwrap().starts_with("mvk_"));
            // The overlap between what it asked for and what we have.
            assert_eq!(
                body["scope"],
                "orchestration:read orchestration:operate terminal:operate review:write relay:read"
            );
            assert!(body["expires_in"].as_i64().unwrap() > 0);
        }

        #[tokio::test]
        async fn a_code_that_cannot_be_redeemed_is_refused_the_way_t3_reads() {
            // Never minted, already spent, expired — one answer for all three.
            let pairing = test_pairing(None);

            let response = router(state_pairing(pairing))
                .oneshot(mobile_exchange("ABCDEFGHJKLM"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

            let body = json(response).await;
            assert_eq!(body["_tag"], "EnvironmentAuthInvalidError");
            assert_eq!(body["code"], "auth_invalid");
            assert_eq!(body["reason"], "invalid_credential");
            assert!(!body["traceId"].as_str().unwrap().is_empty());
        }

        #[tokio::test]
        async fn a_grant_this_environment_does_not_offer_never_reaches_the_code() {
            let response = router(state_pairing(test_pairing(Some(pairing_code(
                "ABCDEFGHJKLM",
            )))))
                .oneshot(exchange(
                    "grant_type=authorization_code&subject_token=ABCDEFGHJKLM\
                     &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
                     &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token",
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);

            let body = json(response).await;
            assert_eq!(body["_tag"], "EnvironmentRequestInvalidError");
            assert_eq!(body["reason"], "invalid_command");
        }

        #[tokio::test]
        async fn a_paired_client_trades_its_token_for_a_ticket() {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/auth/websocket-ticket")
                .body(Body::empty())
                .unwrap();
            request.extensions_mut().insert(viewer());

            let response = router(state()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);

            let body = json(response).await;
            let claims =
                verify_ws_ticket_jwt(body["ticket"].as_str().unwrap(), TEST_SECRET).unwrap();
            assert_eq!(claims.sub, viewer().user_id);
            // T3 decodes this as a `DateTimeUtc`, so it has to be an instant.
            assert!(
                body["expiresAt"]
                    .as_str()
                    .unwrap()
                    .parse::<chrono::DateTime<Utc>>()
                    .is_ok()
            );
        }

        #[tokio::test]
        async fn a_deployment_with_no_signing_secret_says_so() {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/auth/websocket-ticket")
                .body(Body::empty())
                .unwrap();
            request.extensions_mut().insert(viewer());

            let unsigned = UiRpcState {
                session_jwt_secret: None,
                ..state()
            };
            let response = router(unsigned).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(
                json(response).await["reason"],
                "websocket_ticket_issuance_failed"
            );
        }
    }

    mod trace_context {
        use super::*;
        use opentelemetry::trace::{TraceContextExt, TracerProvider};
        use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
        use tracing_subscriber::layer::SubscriberExt;

        const VALID_TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
        const VALID_SPAN_ID: &str = "b7ad6b7169203331";

        #[tokio::test]
        async fn otlp_traces_enabled_reflects_whether_export_is_active() {
            let off = state();
            assert_eq!(
                server_config(&off, &viewer()).await["observability"]["otlpTracesEnabled"],
                false
            );

            let on = UiRpcState {
                rpc: Arc::new(UiRpcConfig {
                    otlp_traces_enabled: true,
                    ..(*off.rpc).clone()
                }),
                ..off
            };
            assert_eq!(
                server_config(&on, &viewer()).await["observability"]["otlpTracesEnabled"],
                true
            );
        }

        #[test]
        fn well_formed_client_context_parses_into_a_remote_span_context() {
            let cx = remote_span_context(Some(VALID_TRACE_ID), Some(VALID_SPAN_ID), Some(true))
                .expect("well-formed ids parse");
            let span_context = cx.span().span_context().clone();
            assert_eq!(span_context.trace_id().to_string(), VALID_TRACE_ID);
            assert_eq!(span_context.span_id().to_string(), VALID_SPAN_ID);
            assert!(span_context.is_sampled());
            assert!(span_context.is_remote());
        }

        #[test]
        fn unsampled_client_context_carries_no_sampled_flag() {
            let cx = remote_span_context(Some(VALID_TRACE_ID), Some(VALID_SPAN_ID), Some(false))
                .expect("well-formed ids parse");
            assert!(!cx.span().span_context().is_sampled());
        }

        #[test]
        fn missing_ids_yield_no_remote_context() {
            assert!(remote_span_context(None, Some(VALID_SPAN_ID), Some(true)).is_none());
            assert!(remote_span_context(Some(VALID_TRACE_ID), None, Some(true)).is_none());
        }

        #[test]
        fn malformed_ids_yield_no_remote_context() {
            // Wrong length.
            assert!(
                remote_span_context(Some("deadbeef"), Some(VALID_SPAN_ID), Some(true)).is_none()
            );
            // Right length, not hex.
            assert!(
                remote_span_context(
                    Some("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
                    Some(VALID_SPAN_ID),
                    Some(true)
                )
                .is_none()
            );
            // The all-zero trace id is explicitly invalid per the W3C spec.
            assert!(
                remote_span_context(
                    Some("00000000000000000000000000000000"),
                    Some(VALID_SPAN_ID),
                    Some(true)
                )
                .is_none()
            );
        }

        #[test]
        fn a_well_formed_client_trace_id_becomes_the_request_spans_trace_id() {
            let (provider, exporter) = {
                let exporter = InMemorySpanExporter::default();
                let provider = SdkTracerProvider::builder()
                    .with_simple_exporter(exporter.clone())
                    .build();
                (provider, exporter)
            };
            let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
            let subscriber = tracing_subscriber::registry().with(otel_layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            {
                let span = request_root_span(
                    "shell.list",
                    Some(VALID_TRACE_ID),
                    Some(VALID_SPAN_ID),
                    Some(true),
                );
                let _entered = span.enter();
            }
            provider.force_flush().expect("flush the simple exporter");

            let spans = exporter.get_finished_spans().expect("exporter is readable");
            assert_eq!(spans.len(), 1);
            assert_eq!(spans[0].span_context.trace_id().to_string(), VALID_TRACE_ID);
        }

        #[test]
        fn a_malformed_client_trace_id_mints_a_fresh_root_and_records_the_raw_value() {
            let exporter = InMemorySpanExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
            let subscriber = tracing_subscriber::registry().with(otel_layer);
            let _guard = tracing::subscriber::set_default(subscriber);

            {
                let span = request_root_span("shell.list", Some("not-a-trace-id"), None, None);
                let _entered = span.enter();
            }
            provider.force_flush().expect("flush the simple exporter");

            let spans = exporter.get_finished_spans().expect("exporter is readable");
            assert_eq!(spans.len(), 1);
            // A fresh root never shares the malformed client value verbatim as
            // its trace id, but the raw value is preserved as an attribute.
            assert_ne!(
                spans[0].span_context.trace_id().to_string(),
                "not-a-trace-id"
            );
            let recorded = spans[0]
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == "client_trace_id");
            assert_eq!(
                recorded.map(|kv| kv.value.as_str().to_string()),
                Some("not-a-trace-id".to_string())
            );
        }
    }
}
