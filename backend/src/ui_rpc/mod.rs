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

pub mod dao;
mod feed;
mod projection;

pub use feed::ShellFeed;

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::agent::catalog::AgentKind;
use crate::auth::identity::AuthIdentity;
use crate::auth::models::UserRole;
use crate::task_listing::{self, EnvironmentIdentity, Resume};
use crate::ui_rpc::dao::ConversationDao;
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
    Subdomain { prefix: String, suffix: String },
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
    workspaces: Arc<dyn WorkspaceDao>,
    conversations: Arc<dyn ConversationDao>,
    model_defaults: Arc<AgentModelDefaults>,
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
    session_cookie_name: String,
    workspaces: Arc<dyn WorkspaceDao>,
    conversations: Arc<dyn ConversationDao>,
    model_defaults: AgentModelDefaults,
) -> Result<Option<Router>, UiRpcConfigError> {
    if !env_enabled("T3_UI_RPC_ENABLED")? {
        return Ok(None);
    }

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
        workspaces,
        conversations,
        model_defaults: Arc::new(model_defaults),
    })))
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
        .route("/api/auth/session", get(auth_session))
        .route("/api/orchestration/shell", get(shell_snapshot_http))
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
) -> Json<Value> {
    debug!(user_id = %identity.user_id, "T3 UI session bootstrap");
    Json(session_state(&state))
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

/// Assemble the shell listing one viewer sees.
///
/// This is the module's data-access seam, and the whole of its internal
/// interface: read the conversations this viewer follows, read the workspaces
/// they need in order to be grouped, and project both onto the wire.
///
/// The two halves are filtered by deliberately different rules, and the
/// asymmetry is load-bearing rather than an oversight. Conversations are the
/// ones the viewer **follows** and may still read — a sidebar is about
/// involvement, so an administrator's is what they follow like everyone else's.
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
        value: projection::shell_snapshot(
            &workspaces,
            &conversations,
            model_defaults,
            sequence,
        ),
        sequence,
        identity: head.identity,
    })
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
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&headers, &state.rpc.allowed_origins) {
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

fn origin_allowed(headers: &HeaderMap, allowed_origins: &[AllowedOrigin]) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| allowed_origins.iter().any(|allowed| allowed.matches(origin)))
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

/// Read a wire request id, which T3 encodes as `string | number`.
///
/// `RpcMessage.RequestEncoded` widened its id to accept either, and the client
/// sends whichever the id already was — an unquoted integer for the counter it
/// generates. The decimal string is what the rest of this protocol correlates
/// on and what a response carries back, so a number becomes one here.
fn deserialize_request_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct RequestIdVisitor;

    impl serde::de::Visitor<'_> for RequestIdVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a request id as a string or an integer")
        }

        fn visit_str<E: serde::de::Error>(self, id: &str) -> Result<String, E> {
            Ok(id.to_string())
        }

        fn visit_u64<E: serde::de::Error>(self, id: u64) -> Result<String, E> {
            Ok(id.to_string())
        }

        fn visit_i64<E: serde::de::Error>(self, id: i64) -> Result<String, E> {
            Ok(id.to_string())
        }
    }

    // A fractional id reaches no visitor here and is refused, which is what
    // `valid_request_id` would decide about it anyway.
    deserializer.deserialize_any(RequestIdVisitor)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
enum ClientFrame {
    Request {
        #[serde(deserialize_with = "deserialize_request_id")]
        id: String,
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
        #[serde(deserialize_with = "deserialize_request_id")]
        request_id: String,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        #[serde(deserialize_with = "deserialize_request_id")]
        request_id: String,
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
                    if !valid_request_id(&request_id) {
                        output
                            .frames
                            .push(connection_defect("Ack requestId is not a decimal string"));
                        continue;
                    }
                    if let Some(stream) = self.streams.get_mut(&request_id) {
                        stream.awaiting_ack = false;
                    }
                }
                ClientFrame::Interrupt { request_id } => {
                    if !valid_request_id(&request_id) {
                        output.frames.push(connection_defect(
                            "Interrupt requestId is not a decimal string",
                        ));
                        continue;
                    }
                    self.streams.remove(&request_id);
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
        id: String,
        tag: String,
        payload: Value,
        state: &UiRpcState,
        identity: &AuthIdentity,
    ) -> Vec<Value> {
        if !valid_request_id(&id) {
            return vec![connection_defect(
                "Request id is not a decimal string and cannot be correlated",
            )];
        }
        if self.streams.contains_key(&id) {
            return vec![connection_defect(format!(
                "Duplicate in-flight RPC request id: {id}"
            ))];
        }

        match tag.as_str() {
            "server.probe" => vec![success_exit(&id, json!({}))],
            "server.getConfig" => vec![success_exit(&id, server_config(state))],
            "server.getSettings" => vec![success_exit(&id, json!({}))],
            "server.refreshProviders" => {
                vec![success_exit(&id, json!({ "providers": [] }))]
            }
            "server.discoverSourceControl" => vec![success_exit(
                &id,
                json!({
                    "versionControlSystems": [],
                    "sourceControlProviders": []
                }),
            )],
            "preview.list" => {
                vec![success_exit(&id, json!({ "sessions": [] }))]
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
                let started = self.start_stream(&id, Some(values));
                if let Some(stream) = self.streams.get_mut(&id) {
                    stream.producer = Some(producer);
                } else {
                    // The stream limit refused it; nothing will read its frames.
                    producer.abort();
                }
                started
            }
            "subscribeServerConfig" => self.start_stream(
                &id,
                Some(vec![json!({
                    "version": 1,
                    "type": "snapshot",
                    "config": server_config(state)
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
            "subscribeVcsStatus"
            | "subscribeTerminalEvents"
            | "subscribeTerminalMetadata"
            | "subscribePreviewEvents"
            | "subscribeDiscoveredLocalServers" => self.start_stream(&id, None),
            _ => vec![unsupported_exit(&id, &tag)],
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
        request_id: String,
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
                "requestId": request_id,
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

    fn start_stream(&mut self, id: &str, values: Option<Vec<Value>>) -> Vec<Value> {
        if self.streams.len() >= MAX_ACTIVE_STREAMS {
            return vec![failure_exit(
                id,
                format!("At most {MAX_ACTIVE_STREAMS} RPC streams may be active"),
            )];
        }

        let awaiting_ack = values.is_some();
        self.streams.insert(
            id.to_string(),
            StreamState {
                awaiting_ack,
                producer: None,
            },
        );
        values
            .map(|values| {
                vec![json!({
                    "_tag": "Chunk",
                    "requestId": id,
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
            "threadSettlement": false,
            "threadSnooze": false
        }
    })
}

fn auth_descriptor(state: &UiRpcState) -> Value {
    json!({
        "policy": "remote-reachable",
        "bootstrapMethods": [],
        "sessionMethods": ["browser-session-cookie"],
        "sessionCookieName": state.session_cookie_name.as_ref()
    })
}

fn session_state(state: &UiRpcState) -> Value {
    json!({
        "authenticated": true,
        "auth": auth_descriptor(state),
        "scopes": [
            "orchestration:read",
            "access:read",
            "relay:read"
        ],
        "sessionMethod": "browser-session-cookie"
    })
}

fn server_config(state: &UiRpcState) -> Value {
    json!({
        "environment": descriptor(&state.rpc),
        "auth": auth_descriptor(state),
        "cwd": "/moatless",
        "keybindingsConfigPath": "/moatless/keybindings.json",
        "keybindings": [],
        "issues": [],
        "providers": [],
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

fn success_exit(request_id: &str, value: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id,
        "exit": {
            "_tag": "Success",
            "value": value
        }
    })
}

fn failure_exit(request_id: &str, defect: impl Into<String>) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id,
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Die",
                "defect": defect.into()
            }]
        }
    })
}

/// The client is told the listing could not be built; the cause is logged once
/// here rather than shipped to the browser.
fn shell_failure_exit(request_id: &str, error: &sqlx::Error, identity: &AuthIdentity) -> Value {
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

fn unsupported_exit(request_id: &str, tag: &str) -> Value {
    failure_exit(
        request_id,
        format!("RPC method is not implemented by the phase-one Moatless server: {tag}"),
    )
}

fn interrupted_exit(request_id: &str) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id,
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
    use uuid::Uuid;

    /// The stub's database. Every read in these tests reports it, so nothing
    /// here exercises the identity guard — that needs two real databases, and
    /// is in `backend/tests/ui_rpc_shell_test.rs`.
    fn stub_identity() -> EnvironmentIdentity {
        EnvironmentIdentity::new("7667539899605483559:16384")
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
        UiRpcState {
            rpc: Arc::new(UiRpcConfig {
                environment_id: environment_id("moatless-test", &stub_identity()),
                label: "Moatless Test".into(),
                server_version: "0.0.28".into(),
                allowed_origins: vec![AllowedOrigin::parse("http://localhost:5733")],
            }),
            environment: Arc::new(stub_identity()),
            session_cookie_name: Arc::from("moatless_session"),
            workspaces: Arc::new(workspaces),
            conversations: Arc::new(conversations),
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
    async fn a_numeric_request_id_is_answered_as_a_decimal_string() {
        // What the T3 client actually puts on the wire: `"id": 21`, unquoted.
        let frame = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": 21,
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
            output.frames[0]["requestId"], "21",
            "a numeric id is correlated as the decimal string a response carries"
        );
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
            let defected = output
                .frames
                .iter()
                .any(|frame| frame["_tag"] == "Defect");
            assert!(!defected, "{tag} with a numeric id should parse, got {output:?}");
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
            json!([])
        );
        assert_eq!(
            output.frames[0]["exit"]["value"]["auth"]["sessionMethods"],
            json!(["browser-session-cookie"])
        );
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
}
