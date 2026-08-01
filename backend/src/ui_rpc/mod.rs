//! Opt-in T3 Code UI compatibility routes.
//!
//! This module is the anti-corruption layer between T3's contract and Moatless
//! state: it is the only place in the backend that knows T3's vocabulary, and
//! that vocabulary appears on the wire and nowhere behind it. The shell listing
//! reads the conversations a viewer follows and the workspaces they run
//! against, projects both onto T3's shapes, and reports the position it read
//! them at. A subscription then watches the listing change feed and sends each
//! changed conversation's row, re-read and whole.
//!
//! A client that reconnects passes the position it already holds —
//! `afterSequence` — and is replayed from it rather than rebuilt, for as long
//! as the changes it missed are still retained. Past that it is handed a fresh
//! listing, which is a different member of the contract's stream union and not
//! an empty answer. Only turn transitions enter the feed so far; writes remain
//! unimplemented.
//!
//! Opening one conversation reads its transcript and projects it into the two
//! collections T3 renders — what was *said* as messages, what was *done* as a
//! work log — over the same route in both directions: an HTTP snapshot the
//! client hydrates from, and a subscription that keeps it current. That
//! subscription reads a change feed of its own (`task_activities`) and sends
//! the contract's own events, so a conversation that moves costs the row that
//! moved rather than the whole transcript. It resumes on the same terms as the
//! shell's, in its own numbering: a conversation's positions are not the
//! listing's, and a cursor from one is refused by the other. See
//! [`ThreadFeed`].

mod auth;
pub mod backend;
mod commands;
pub mod dao;
mod feed;
pub mod files;
pub mod ports;
mod preview;
pub mod preview_tabs;
mod projection;
mod providers;
mod review;
mod servers;
pub mod workspace;

pub use feed::{ShellFeed, ThreadFeed};

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
use tracing::{debug, error, info, warn};

use crate::agent::catalog::AgentKind;
use crate::auth::identity::{AuthIdentity, AuthMethod};
use crate::auth::models::UserRole;
use crate::auth::pairing::DevicePairing;
use crate::sandbox::access::{SandboxConnector, SandboxIntentReader, SandboxObserver};
use crate::task::dao::TaskDao;
use crate::task_activities::{self, ConversationHead};
use crate::task_listing::{self, EnvironmentIdentity, Resume};
use crate::ui_rpc::dao::ConversationDao;
use crate::ui_rpc::preview::{PreviewError, PreviewState};
use crate::ui_rpc::preview_tabs::PreviewTabDao;
use crate::ui_rpc::servers::{ServersError, ServersState};
use crate::ui_rpc::workspace::WorkspaceAccessError;
use crate::workspace::dao::WorkspaceDao;
use moatless_backend_core::scope::ScopeContext;

const MAX_FRAME_BYTES: usize = 1024 * 1024;
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

#[derive(Debug, Clone)]
struct UiRpcConfig {
    environment_id: String,
    label: String,
    server_version: String,
    allowed_origins: Vec<AllowedOrigin>,
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
    pub(super) fn for_kind(&self, kind: AgentKind) -> Option<&str> {
        match kind {
            AgentKind::ClaudeCode => self.claude_code.as_deref(),
            AgentKind::Codex => self.codex.as_deref(),
        }
    }
}

#[derive(Clone)]
struct UiRpcState {
    rpc: Arc<UiRpcConfig>,
    /// The database this process started against, and therefore the database
    /// every position it has handed out was allocated in. Compared against what
    /// the database says it is on each resume; see [`task_listing::resume`].
    environment: Arc<EnvironmentIdentity>,
    session_cookie_name: Arc<str>,
    /// Signs WebSocket tickets. Absent when the deployment configures no JWT
    /// secret, which is the same condition under which session cookies do not
    /// work either; the ticket route then answers that it cannot issue one.
    session_jwt_secret: Option<Arc<str>>,
    /// Turns a pairing code into the token a client on another device keeps.
    /// The issuing half lives with the rest of the account routes.
    pairing: Arc<DevicePairing>,
    workspaces: Arc<dyn WorkspaceDao>,
    conversations: Arc<dyn ConversationDao>,
    /// How a conversation's files are reached. The only piece of Moatless
    /// behind this module that is not a database read: a checkout exists inside
    /// its Sandbox and nowhere else.
    sandboxes: Arc<dyn SandboxConnector>,
    /// What a Task's servers are doing. Distinct from `sandboxes`, which
    /// reaches *into* a Sandbox; this only observes one from outside.
    servers: ServersState,
    /// The browser tabs open on a Task, and the fan-out that tells every
    /// replica when one changes.
    preview: PreviewState,
    model_defaults: Arc<AgentModelDefaults>,
    /// The agent catalog, as this deployment can actually run it. Read on every
    /// config request because credentials come and go without a restart.
    agents: Arc<dyn ports::AgentCatalogSource>,
    /// The one thing this module does that is not a read.
    commands: Arc<dyn ports::ThreadCommandSink>,
}

/// Build the T3 compatibility router when its environment gate is enabled.
///
/// Configuration stays at this HTTP boundary rather than becoming part of
/// sandbox or pod configuration. The global Moatless auth middleware protects
/// every route except the public discovery descriptor.
///
/// The one thing here that is *not* configuration is the environment's
/// identity, which is read from the database. See [`environment_id`].
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
        tasks,
        intent,
        preview_tabs,
        model_defaults,
        agents,
        commands,
    } = dependencies;
    let environment = conversations.listing_head().await?.identity;
    let name = env_value("T3_UI_RPC_ENVIRONMENT_ID", "moatless");
    let config = UiRpcConfig {
        environment_id: environment_id(&name, &environment),
        label: env_value("T3_UI_RPC_LABEL", "Moatless"),
        server_version: env_value("T3_UI_RPC_SERVER_VERSION", env!("CARGO_PKG_VERSION")),
        allowed_origins: env::var("T3_UI_RPC_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(AllowedOrigin::parse)
            .collect(),
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
        session_jwt_secret: session_jwt_secret.map(Arc::from),
        pairing,
        workspaces,
        conversations,
        sandboxes,
        servers: ServersState {
            observer,
            tasks: tasks.clone(),
            intent,
        },
        preview: PreviewState {
            tabs: preview_tabs,
            tasks,
        },
        model_defaults: Arc::new(model_defaults),
        agents,
        commands,
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
    /// cookies use, read through [`crate::auth::jwt::get_jwt_secret`] so the
    /// sandbox-secret fallback applies here too.
    pub session_jwt_secret: Option<String>,
    pub pairing: Arc<DevicePairing>,
    pub workspaces: Arc<dyn WorkspaceDao>,
    pub conversations: Arc<dyn ConversationDao>,
    /// Reaches a conversation's Sandbox, which is where its checkout lives.
    pub sandboxes: Arc<dyn SandboxConnector>,
    /// Reports what a Task's servers are doing, without reaching inside.
    pub observer: Arc<dyn SandboxObserver>,
    /// Authorizes every `servers.*` and `preview.*` call: a scoped read that
    /// finds nothing is the same answer as a Task that does not exist.
    pub tasks: Arc<dyn TaskDao>,
    /// The desired state a status read is resolved against.
    pub intent: Arc<dyn SandboxIntentReader>,
    pub preview_tabs: Arc<dyn PreviewTabDao>,
    pub model_defaults: AgentModelDefaults,
    /// Which agents this viewer can run, and with which models.
    pub agents: Arc<dyn ports::AgentCatalogSource>,
    /// Carries out the commands the T3 client dispatches.
    pub commands: Arc<dyn ports::ThreadCommandSink>,
}

/// The identity this deployment presents itself by: a configured name, and a
/// digest of the database behind it.
///
/// **This is the half of the spec's Decision 7 that does not run on this side
/// of the wire.** T3 keys everything it caches for an environment — the shell
/// listing included — on this string
/// (`client-runtime/src/platform/persistence.ts:59-63`, read at
/// `state/shell.ts:56-70` before the first frame arrives). What a client does
/// when the string changes depends on how it reached us, and it is worth being
/// exact about both, because only one of them is silent:
///
/// - **Same-origin, which is how Moatless is reached.** The web application
///   builds its primary connection *from* the descriptor — it does not hold an
///   id and check ours against it (`apps/web/src/connection/platform.ts:286-300`).
///   So a changed id becomes the client's id, no cached listing exists under
///   it, and the shell starts empty. Nothing is sent and nothing the client
///   holds has to be believed.
/// - **Registered connections — bearer, relay, SSH.** These stored our id at
///   onboarding and compare it against the descriptor on every connection
///   attempt, refusing to connect on a mismatch:
///   *"Connected environment X does not match Y."*
///   (`client-runtime/src/connection/errors.ts:25-33`, reached from
///   `authorization/service.ts:114` and `connection/resolver.ts:106`.) A
///   database replacement therefore requires re-onboarding such a connection.
///   That is loud rather than silent, which is the property this ticket is
///   about; it is not free, and it is written down here rather than discovered
///   during an incident.
///
/// The id is read once per page load and held for the session, so it moves at
/// reload, not at reconnect. This is why it is the outer layer and not the
/// mechanism: the guards in [`task_listing::resume`] and
/// [`ShellFeed::poll`](feed::ShellFeed::poll) are what actually make a
/// mid-session replacement safe, and they run entirely on this side. The
/// property the identity buys on top of them is that a mismatch after a restore
/// — which from the client's side is indistinguishable from a first connection
/// — *becomes* a first connection, rather than a claim the client is asked to
/// check about itself.
///
/// `T3_UI_RPC_ENVIRONMENT_ID` survives as the readable half. Its meaning has
/// narrowed: it names the deployment, and it no longer decides what makes one
/// environment different from another. Two deployments sharing a database now
/// present as one environment however they are configured, which is the truth —
/// they have the same conversations and the same positions.
fn environment_id(name: &str, identity: &EnvironmentIdentity) -> String {
    format!("{}-{}", name.trim(), identity.digest())
}

fn router(state: UiRpcState) -> Router {
    Router::new()
        .route("/.well-known/t3/environment", get(environment_descriptor))
        .route("/oauth/token", post(auth::token_exchange))
        .route("/api/auth/websocket-ticket", post(auth::websocket_ticket))
        .route("/api/auth/session", get(auth_session))
        .route("/api/orchestration/shell", get(shell_snapshot_http))
        .route(
            "/api/orchestration/threads/{thread_id}",
            get(thread_snapshot_http),
        )
        .route("/ws", get(websocket_upgrade))
        .with_state(state)
}

impl UiRpcConfig {
    fn validate(&self) -> Result<(), UiRpcConfigError> {
        if self.environment_id.trim().is_empty()
            || self.label.trim().is_empty()
            || self.server_version.trim().is_empty()
        {
            return Err(UiRpcConfigError::Invalid(
                "T3_UI_RPC_ENVIRONMENT_ID, T3_UI_RPC_LABEL, and \
                 T3_UI_RPC_SERVER_VERSION must be non-empty"
                    .into(),
            ));
        }
        if self.allowed_origins.is_empty() {
            return Err(UiRpcConfigError::Invalid(
                "T3_UI_RPC_ALLOWED_ORIGINS must contain at least one origin".into(),
            ));
        }
        Ok(())
    }
}

/// Whether this deployment mounts the T3 compatibility routes.
///
/// Read by anything that has to agree with [`router_from_env`] about that —
/// the feature flag the settings dialog reads being the one, so that "pair a
/// device" appears exactly when the endpoints it drives exist. It goes through
/// the same parser rather than a second reading of the same variable, because
/// a second one would disagree: `T3_UI_RPC_ENABLED=on` mounts the routes, and
/// a stricter reader would hide the only way to use them.
///
/// A value that is neither is `false` here and a startup failure there. That
/// asymmetry is fine: nothing reaches this if the process did not start.
pub fn is_enabled() -> bool {
    env_enabled("T3_UI_RPC_ENABLED").unwrap_or(false)
}

fn env_enabled(key: &str) -> Result<bool, UiRpcConfigError> {
    match env::var(key) {
        Err(env::VarError::NotPresent) => Ok(false),
        Err(error) => Err(UiRpcConfigError::Invalid(format!(
            "{key} could not be read: {error}"
        ))),
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" | "" => Ok(false),
            _ => Err(UiRpcConfigError::Invalid(format!(
                "{key} must be true or false"
            ))),
        },
    }
}

fn env_value(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

async fn environment_descriptor(State(state): State<UiRpcState>) -> Json<Value> {
    Json(descriptor(&state.rpc))
}

async fn auth_session(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<AuthIdentity>,
    method: Option<Extension<AuthMethod>>,
) -> Json<Value> {
    debug!(user_id = %identity.user_id, "T3 UI session bootstrap");
    Json(session_state(&state, method.map(|Extension(m)| m)))
}

async fn shell_snapshot_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<AuthIdentity>,
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
    Extension(identity): Extension<AuthIdentity>,
    Path(thread_id): Path<String>,
) -> Response {
    debug!(user_id = %identity.user_id, task_id = %thread_id, "T3 UI thread snapshot bootstrap");
    match thread_snapshot(
        state.workspaces.as_ref(),
        state.conversations.as_ref(),
        &state.model_defaults,
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
    workspaces: &dyn WorkspaceDao,
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

/// Assemble one conversation as the thread view carries it.
///
/// `Ok(None)` is every way a conversation can be absent for this viewer —
/// deleted, closed, unfollowed, no longer readable, or grouped under a workspace
/// that is not listable — collapsed into one answer, because they are one answer
/// to the client and distinguishing them would disclose which.
///
/// The conversation row is resolved through the **same narrowed listing query
/// the shell uses**, so the two cannot drift: a conversation the sidebar shows
/// opens, and one it does not is not reachable by typing its id into the
/// address bar. Following is part of that rule, deliberately — a sidebar is
/// about involvement, and this is the view its rows open into. So is being open:
/// a closed Task is read in Moatless's own UI, and offering it here would be a
/// conversation T3 can display and never send to.
///
/// # Read order
///
/// The conversation's change position comes before the rows, for the reason
/// given on [`shell_snapshot`]: read afterwards it would sit above a change the
/// rows already reflect, and the event carrying that change would be discarded
/// by the client as stale — one conversation frozen, silently. Read first, the
/// worst case is a change delivered twice, and every event this feed sends is
/// an upsert, so twice is the same as once.
pub async fn thread_snapshot(
    workspaces: &dyn WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Option<ThreadSnapshot>, sqlx::Error> {
    let head = conversations.conversation_head(task_id).await?;
    let rows = conversations
        .list_followed_by_ids(viewer, &[task_id.to_string()])
        .await?;
    let Some(conversation) = rows.into_iter().next() else {
        return Ok(None);
    };
    let referenced: Vec<String> = conversation.workspace_id.clone().into_iter().collect();
    let workspaces = workspaces
        .list_readable_including(viewer, &referenced)
        .await?;
    let project_id = conversation
        .workspace_id
        .as_deref()
        .filter(|id| workspaces.iter().any(|listed| listed.workspace.id == *id));
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
                model_defaults,
                &transcript,
            )
        }),
        head,
    }))
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
/// namespace. [`task_activities::resume`] is what decides between them, on the
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
    workspaces: &dyn WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    task_id: &str,
    after: Option<i64>,
) -> Result<Option<(Vec<Value>, ThreadFeed)>, sqlx::Error> {
    let head = conversations.conversation_head(task_id).await?;
    // Qualified: the shell has a decision of the same shape over a different
    // log, and the two cursors are not interchangeable.
    let resume = match after {
        Some(after) => task_activities::resume(head, after),
        None => task_activities::Resume::Rebuild,
    };
    if let task_activities::Resume::From(cursor) = resume {
        // A replay reads nothing when nothing changed, and a poll that reads
        // nothing checks nothing. So the visibility rule is applied here as
        // well, or subscribing to a conversation this viewer may not see would
        // succeed for as long as it stayed still.
        if conversations
            .list_followed_by_ids(viewer, &[task_id.to_string()])
            .await?
            .is_empty()
        {
            return Ok(None);
        }
        let mut feed = ThreadFeed::resuming_from(task_id.to_string(), cursor);
        let Some(values) = feed.poll(conversations, viewer).await? else {
            return Ok(None);
        };
        debug!(
            user_id = %viewer.user_id,
            task_id,
            cursor,
            replayed = values.len(),
            "T3 UI thread subscription resumed from the client's position"
        );
        return Ok(Some((values, feed)));
    }

    let Some(snapshot) =
        thread_snapshot(workspaces, conversations, model_defaults, viewer, task_id).await?
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
    Ok(Some((
        vec![json!({
            "kind": "snapshot",
            "snapshot": snapshot.value
        })],
        feed,
    )))
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
    pub identity: EnvironmentIdentity,
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
/// it is [`task_listing::resume`]'s third reason to rebuild, and the only one
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
/// So an un-replayable gap is never an empty answer. It is the same frame a
/// client with no snapshot at all receives, and the reference server resolves
/// it identically (`apps/server/src/ws.ts:1297-1305`: a gap past
/// `SHELL_RESUME_MAX_GAP`, or a cursor ahead of the head, returns
/// `Stream.make({ kind: "snapshot" })` instead of the replay).
///
/// # Why the catch-up is one ordinary poll
///
/// Replay reads through [`ShellFeed::poll`] — the same call the live
/// subscription makes every interval, with the same coalescing, the same
/// re-read of each whole row, and the same two access filters. A resuming
/// client is therefore told exactly what a client that never disconnected would
/// have been told, and there is no second delivery path to drift from the first.
///
/// The poll's own cap is what bounds the catch-up, and there is no window in
/// which a change can be lost between the two: the feed's cursor advances only
/// over records it read, and the same feed continues live. Anything left over
/// arrives on the next poll.
pub async fn shell_subscription(
    workspaces: &dyn WorkspaceDao,
    conversations: &dyn ConversationDao,
    model_defaults: &AgentModelDefaults,
    viewer: &ScopeContext,
    environment: &EnvironmentIdentity,
    after: Option<i64>,
) -> Result<ShellSubscription, sqlx::Error> {
    let head = conversations.listing_head().await?;
    let resume = match after {
        Some(after) => task_listing::resume(environment, &head.identity, head.position, after),
        None => Resume::Rebuild,
    };
    match resume {
        Resume::From(cursor) => {
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
        Resume::Rebuild => {
            let snapshot =
                shell_snapshot(workspaces, conversations, model_defaults, viewer).await?;
            if let Some(after) = after {
                info!(
                    user_id = %viewer.user_id,
                    after,
                    head = snapshot.sequence,
                    retained = task_listing::RETAINED_CHANGES,
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
/// # Why the viewer's readable projects are a sound answer, and their listing
/// is not
///
/// A resuming client is not holding nothing. It re-fetches the whole shell
/// snapshot over HTTP on every resubscribe and applies it *before* it
/// subscribes, and the `afterSequence` it sends is that snapshot's own sequence
/// (`client-runtime/src/state/shell.ts:204-212`). So its projects are the ones
/// [`shell_snapshot`] returned moments ago, and [`shell_snapshot`] returns
/// **every workspace the viewer may read**, unconditionally, plus the ones a
/// listed conversation references.
///
/// This reads back the first half and not the second, and the asymmetry is the
/// point:
///
/// - A project the viewer may read now was in that snapshot unless it *became*
///   readable in between — and becoming readable is a workspace mutation, which
///   records its own listing change (`workspace/dao.rs`). The catch-up poll
///   replays that record as a `project-upserted` before anything can be
///   delivered into the project, so the client is given the group either way.
/// - A project held only because a conversation *references* it is deliberately
///   left out. That is precisely the case where a conversation can arrive under
///   a project the client has never been sent, and it is what
///   `ShellFeed::resynchronize` exists to answer with a whole listing.
///
/// What this does not cover is the viewer's own scope widening between their
/// snapshot request and their subscribe — no workspace changed, so no record
/// exists. The window is one round trip, and the cost is one row not rendered
/// until the next rebuild rather than anything disclosed.
async fn resume_grouping(
    workspaces: &dyn WorkspaceDao,
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

/// The archived half of the listing: the viewer's projects, and nothing else.
///
/// The client reads this as *the conversations whose `archivedAt` is set*, and
/// this module never sets one — closing a conversation in Moatless records no
/// timestamp of its own, and the contract's own empty value is what the row
/// carries. Returning the live listing here would present every conversation a
/// person follows as archived.
async fn archived_shell_snapshot(
    workspaces: &dyn WorkspaceDao,
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
) -> Result<Value, sqlx::Error> {
    let sequence = conversations.listing_head().await?.position;
    let workspaces = workspaces.list_readable_including(viewer, &[]).await?;
    Ok(projection::shell_snapshot(
        &workspaces,
        &[],
        &AgentModelDefaults::default(),
        sequence,
    ))
}

/// The access scope the session's viewer reads Moatless rows with.
fn viewer_scope(identity: &AuthIdentity) -> ScopeContext {
    ScopeContext {
        user_id: identity.user_id.to_string(),
        is_admin: matches!(identity.role, UserRole::Admin),
    }
}

async fn websocket_upgrade(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<AuthIdentity>,
    method: Option<Extension<AuthMethod>>,
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
fn origin_checked(method: Option<Extension<AuthMethod>>) -> bool {
    !matches!(method, Some(Extension(AuthMethod::WebSocketTicket)))
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
async fn serve_websocket(socket: WebSocket, state: UiRpcState, identity: AuthIdentity) {
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
        identity: &AuthIdentity,
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
                    // Parsing these fields is intentional even though phase one
                    // does not consume request headers or tracing context.
                    drop((headers, trace_id, span_id, sampled));
                    output
                        .frames
                        .extend(self.handle_request(id, tag, payload, state, identity).await);
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
        identity: &AuthIdentity,
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
            "server.discoverSourceControl" => vec![success_exit(
                &id,
                json!({
                    "versionControlSystems": [],
                    "sourceControlProviders": []
                }),
            )],
            "vcs.listRefs" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![success_exit(&id, review::no_refs())];
                };
                match review::list_refs(
                    state.conversations.as_ref(),
                    state.sandboxes.as_ref(),
                    &viewer_scope(identity),
                    &review::RefRequest::from_payload(&cwd, &payload),
                )
                .await
                {
                    Ok(refs) => vec![success_exit(&id, refs)],
                    // A conversation with no Sandbox to ask is not an error the
                    // person should be shown; see [`review::no_refs`].
                    Err(
                        WorkspaceAccessError::NoWorktree
                        | WorkspaceAccessError::Unaddressable
                        | WorkspaceAccessError::NotRunning(_),
                    ) => vec![success_exit(&id, review::no_refs())],
                    Err(error) => vec![review_failure_exit(&id, &tag, &error, identity)],
                }
            }
            "review.getDiffPreview" => {
                let Some(cwd) = workspace_cwd(&payload) else {
                    return vec![review_failure_exit(
                        &id,
                        &tag,
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
                    Err(error) => vec![review_failure_exit(&id, &tag, &error, identity)],
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
                    Err(error) => vec![servers_failure_exit(&id, &error, identity)],
                }
            }
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
            "cloud.getRelayClientStatus" => vec![success_exit(
                &id,
                json!({
                    "status": "unsupported",
                    "platform": platform_os(),
                    "arch": platform_arch(),
                    "version": state.rpc.server_version
                }),
            )],
            "orchestration.getArchivedShellSnapshot" => {
                let viewer = viewer_scope(identity);
                match archived_shell_snapshot(
                    state.workspaces.as_ref(),
                    state.conversations.as_ref(),
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
                    &state.model_defaults,
                    &viewer,
                    &task_id,
                    after_sequence(&payload),
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
                let Some((mut values, feed)) = opened else {
                    return vec![failure_exit(&id, "Conversation not found")];
                };
                if payload
                    .get("requestCompletionMarker")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    // Last, after the snapshot. Until the client sees this it
                    // renders the conversation as still loading however much of
                    // it has arrived — which is what an unimplemented
                    // subscription looked like from the outside.
                    values.push(json!({ "kind": "synchronized" }));
                }
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
            // Every asset T3 asks this environment for is a file on a machine
            // it can reach, and Moatless projects are not directories on this
            // one — a project's root path is synthesized, and the checkout only
            // exists inside a per-task Sandbox. Answering "not found" is the
            // honest projection and the one the client renders (a project
            // without a favicon), where a defect leaves the same blank icon
            // behind a stack trace.
            "assets.createUrl" => {
                // The error carries the resource back, and the client decodes it
                // against the same schema it sent — so a request without one has
                // no typed answer to give and is a malformed request instead.
                let Some(resource) = payload.get("resource") else {
                    return vec![failure_exit(&id, "assets.createUrl requires a resource")];
                };
                vec![typed_failure_exit(
                    &id,
                    json!({
                        "_tag": "AssetWorkspaceAssetNotFoundError",
                        "resource": resource
                    }),
                )]
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
                        "projectName": state.rpc.label
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
            "subscribeTerminalEvents"
            | "subscribeTerminalMetadata"
            | "subscribeDiscoveredLocalServers" => self.start_stream(&id, None),
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
                    // not about this loop, and it was false when it was first
                    // written: the cursor was advanced before the reads that
                    // build the batch, so a failure in any of them skipped that
                    // batch permanently and told nobody. `poll` now advances
                    // last. Do not move it back.
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
            let polled = feed.poll(state.conversations.as_ref(), &viewer).await;
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
    json!({
        "environmentId": config.environment_id,
        "label": config.label,
        "platform": {
            "os": platform_os(),
            "arch": platform_arch()
        },
        "serverVersion": config.server_version,
        "capabilities": {
            "repositoryIdentity": false,
            "connectionProbe": true,
            "threadSettlement": true,
            "threadSnooze": true,
            // Whether `thread.user-input.respond` is understood. A client
            // cannot otherwise tell this server from one that refuses it, and
            // the descriptor is read before any thread is opened.
            "userInputResponse": true
        }
    })
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
fn session_state(state: &UiRpcState, method: Option<AuthMethod>) -> Value {
    json!({
        "authenticated": true,
        "auth": auth_descriptor(state),
        "scopes": auth::GRANTED_SCOPES,
        "sessionMethod": match method {
            Some(AuthMethod::SessionCookie) | None => "browser-session-cookie",
            Some(AuthMethod::ApiKey) | Some(AuthMethod::WebSocketTicket) => "bearer-access-token",
        }
    })
}

/// Now, in the format the contract's `IsoDateTime` decodes.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn server_config(state: &UiRpcState, identity: &AuthIdentity) -> Value {
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
            "otlpTracesEnabled": false,
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
    identity: &AuthIdentity,
) -> Value {
    let error_tag = match tag {
        "projects.readFile" => "ProjectReadFileError",
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
        WorkspaceAccessError::NoWorktree | WorkspaceAccessError::Unaddressable => {
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
    error: &WorkspaceAccessError,
    identity: &AuthIdentity,
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
        WorkspaceAccessError::NoWorktree | WorkspaceAccessError::Unaddressable => {
            debug!(?error, %tag, user_id = %identity.user_id, "T3 UI review request refused");
        }
    }
    typed_failure_exit(
        request_id,
        json!({
            "_tag": "GitCommandError",
            "operation": tag,
            "command": "git",
            "cwd": "",
            "detail": error.message(),
        }),
    )
}

/// The client is told the listing could not be built; the cause is logged once
/// here rather than shipped to the browser.
fn shell_failure_exit(
    request_id: &RequestId,
    error: &sqlx::Error,
    identity: &AuthIdentity,
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

/// The client is told the servers could not be read; the cause is logged once
/// here rather than shipped to the browser.
fn servers_failure_exit(
    request_id: &RequestId,
    error: &ServersError,
    identity: &AuthIdentity,
) -> Value {
    match error {
        ServersError::NotFound => failure_exit(request_id, "Conversation not found"),
        error => {
            error!(%error, user_id = %identity.user_id, "Failed to read a Task's servers");
            failure_exit(request_id, "Failed to read the conversation's servers")
        }
    }
}

fn unsupported_exit(request_id: &RequestId, tag: &str) -> Value {
    failure_exit(
        request_id,
        format!("RPC method is not implemented by the phase-one Moatless server: {tag}"),
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
    use crate::task_listing::ListingHead;
    use crate::ui_rpc::dao::MockConversationDao;
    use crate::workspace::dao::MockWorkspaceDao;
    use async_trait::async_trait;
    use uuid::Uuid;

    /// The stub's database. Every read in these tests reports it, so nothing
    /// here exercises the identity guard — that needs two real databases, and
    /// is in `backend/tests/ui_rpc_shell_test.rs`.
    fn stub_identity() -> EnvironmentIdentity {
        EnvironmentIdentity::new("7667539899605483559:16384")
    }

    /// A deployment whose Sandboxes are all unreachable.
    ///
    /// Every file, server and log request these tests make is refused before it
    /// would reach a Sandbox — by the `cwd`, by a listing that contains no
    /// conversations, or by a scoped Task read that finds nothing — so these
    /// seams exist to be held, not called. What a reachable Sandbox answers is
    /// a gRPC or K8s round trip and belongs in an integration test.
    struct UnreachableSandboxes;

    fn unreachable(task_id: &str) -> crate::sandbox::error::SandboxError {
        crate::sandbox::error::SandboxError::NotReady(task_id.to_string())
    }

    #[async_trait]
    impl SandboxConnector for UnreachableSandboxes {
        async fn sandbox_client(
            &self,
            task_id: &str,
        ) -> Result<crate::sandbox::client::SandboxClient, crate::sandbox::error::SandboxError>
        {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl crate::sandbox::access::SandboxIntentReader for UnreachableSandboxes {
        async fn desired_state(
            &self,
            task_id: &str,
        ) -> Result<crate::task::models::SandboxDesiredState, crate::sandbox::error::SandboxError>
        {
            Err(unreachable(task_id))
        }
    }

    #[async_trait]
    impl SandboxObserver for UnreachableSandboxes {
        async fn sandbox_status(
            &self,
            task_id: &str,
            _task_status: crate::task::models::TaskStatus,
            _desired_state: crate::task::models::SandboxDesiredState,
        ) -> Result<
            crate::sandbox::schemas::SandboxStatusFullResponse,
            crate::sandbox::error::SandboxError,
        > {
            Err(unreachable(task_id))
        }

        async fn live_status(
            &self,
            task_id: &str,
            _task_status: crate::task::models::TaskStatus,
            _desired_state: crate::task::models::SandboxDesiredState,
        ) -> Result<crate::sandbox::live_status::LiveStatus, crate::sandbox::error::SandboxError>
        {
            Err(unreachable(task_id))
        }

        async fn server_logs(
            &self,
            task_id: &str,
            _server_name: &str,
            _options: crate::sandbox::access::LogOptions,
        ) -> Result<
            futures::stream::BoxStream<
                'static,
                Result<String, crate::sandbox::error::SandboxError>,
            >,
            crate::sandbox::error::SandboxError,
        > {
            Err(unreachable(task_id))
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
    }

    /// These tests are about the envelope encoding, so the listing behind it
    /// is a stub. What a viewer actually sees is asserted against a real
    /// database in `backend/tests/ui_rpc_shell_test.rs`.
    fn state() -> UiRpcState {
        let mut workspaces = MockWorkspaceDao::new();
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
        conversations.expect_listing_head().returning(|| {
            Ok(ListingHead {
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
        UiRpcState {
            rpc: Arc::new(UiRpcConfig {
                environment_id: environment_id("moatless-test", &stub_identity()),
                label: "Moatless Test".into(),
                server_version: "0.0.28".into(),
                allowed_origins: vec![AllowedOrigin::parse("http://localhost:5733")],
            }),
            environment: Arc::new(stub_identity()),
            session_cookie_name: Arc::from("moatless_session"),
            session_jwt_secret: Some(Arc::from("test-secret")),
            pairing: Arc::new(DevicePairing::new(
                Arc::new(crate::auth::pairing::MockPairingCodeDao::new()),
                Arc::new(crate::auth::api_keys::MockApiKeyDao::new()),
            )),
            workspaces: Arc::new(workspaces),
            conversations: Arc::new(conversations),
            sandboxes: Arc::new(UnreachableSandboxes),
            servers: ServersState {
                observer: Arc::new(UnreachableSandboxes),
                tasks: Arc::new(crate::task::dao::MockTaskDao::new()),
                intent: Arc::new(UnreachableSandboxes),
            },
            preview: PreviewState {
                tabs: Arc::new(preview_tabs::InMemoryPreviewTabDao::new()),
                tasks: Arc::new(crate::task::dao::MockTaskDao::new()),
            },
            agents: Arc::new(StubAgents),
            commands: Arc::new(UnreachableCommands),
            model_defaults: Arc::new(AgentModelDefaults::default()),
        }
    }

    /// A session whose pushed frames go nowhere. These tests drive the request
    /// side; what a subscription's poller produces is asserted against a real
    /// database in `backend/tests/ui_rpc_shell_test.rs`.
    fn session() -> ProtocolSession {
        ProtocolSession::new(mpsc::channel(OUTBOUND_FRAMES).0)
    }

    fn viewer() -> AuthIdentity {
        AuthIdentity {
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
            environment_id("moatless-test", &stub_identity()),
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

    #[tokio::test]
    async fn interrupt_terminates_active_stream() {
        let mut session = session();
        session
            .handle_json(
                &request("3", "subscribeTerminalEvents", json!({})),
                &state(),
                &viewer(),
            )
            .await;
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
            label: "Moatless".into(),
            server_version: "0.0.28".into(),
            allowed_origins: vec![],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_ticket_holder_needs_no_origin_and_everyone_else_does() {
        assert!(!origin_checked(Some(Extension(
            AuthMethod::WebSocketTicket
        ))));
        assert!(origin_checked(Some(Extension(AuthMethod::SessionCookie))));
        assert!(origin_checked(Some(Extension(AuthMethod::ApiKey))));
        // An upgrade the middleware never saw is treated as the cookie case.
        assert!(origin_checked(None));
    }

    /// The pairing exchange and the ticket route, over HTTP, as a client on
    /// another device drives them.
    mod pairing {
        use super::*;
        use crate::auth::api_keys::MockApiKeyDao;
        use crate::auth::jwt::verify_ws_ticket_jwt;
        use crate::auth::pairing::{MockPairingCodeDao, PairingCode};
        use axum::body::Body;
        use axum::http::Request;
        use chrono::Utc;
        use tower::ServiceExt;

        const TEST_SECRET: &str = "test-secret";

        fn state_pairing(codes: MockPairingCodeDao, api_keys: MockApiKeyDao) -> UiRpcState {
            UiRpcState {
                pairing: Arc::new(DevicePairing::new(Arc::new(codes), Arc::new(api_keys))),
                ..state()
            }
        }

        /// A code that is still live, as `consume_pairing_code` returns one.
        fn live_code() -> PairingCode {
            PairingCode {
                id: "code-1".into(),
                code_hash: "hash".into(),
                user_id: Uuid::nil().to_string(),
                group_id: None,
                label: Some("Bedside phone".into()),
                created_at: Utc::now(),
                expires_at: Utc::now() + chrono::Duration::minutes(5),
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
            let mut codes = MockPairingCodeDao::new();
            codes
                .expect_consume_pairing_code()
                .returning(|_, _| Ok(Some(live_code())));
            let mut api_keys = MockApiKeyDao::new();
            // The name a person gave the code, and the OS the client claims.
            api_keys
                .expect_insert_api_key()
                .withf(|key| key.name == "Bedside phone (iOS)")
                .returning(|_| Ok(()));

            let response = router(state_pairing(codes, api_keys))
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
            assert_eq!(body["scope"], "orchestration:read relay:read");
            assert!(body["expires_in"].as_i64().unwrap() > 0);
        }

        #[tokio::test]
        async fn a_code_that_cannot_be_redeemed_is_refused_the_way_t3_reads() {
            let mut codes = MockPairingCodeDao::new();
            // Never minted, already spent, expired — one answer for all three.
            codes
                .expect_consume_pairing_code()
                .returning(|_, _| Ok(None));

            let response = router(state_pairing(codes, MockApiKeyDao::new()))
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
            let mut codes = MockPairingCodeDao::new();
            codes.expect_consume_pairing_code().never();

            let response = router(state_pairing(codes, MockApiKeyDao::new()))
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
}
