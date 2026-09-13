use crate::authentication::viewer_scope;
use crate::rpc::config::descriptor;
use crate::rpc::config::session_state;
use crate::rpc::socket::MAX_FRAME_BYTES;
use crate::rpc::socket::serve_websocket;
use crate::scripts;
use crate::scripts::ScriptsError;
use crate::snapshots::shell_snapshot;
use crate::snapshots::thread_snapshot;
use crate::state::UiRpcState;
use axum::Json;
use axum::extract::Extension;
use axum::extract::Path;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use moatless_ports::sandbox::SandboxError;
use moatless_types::auth::AllowedOrigin;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use moatless_types::ui_rpc::UiRpcAuthMethod;
use serde_json::Value;
use serde_json::json;
use tracing::debug;
use tracing::error;
use tracing::warn;

pub(crate) async fn environment_descriptor(State(state): State<UiRpcState>) -> Json<Value> {
    Json(descriptor(&state.rpc))
}

pub(crate) async fn auth_session(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
) -> Json<Value> {
    debug!(user_id = %identity.user_id, "T3 UI session bootstrap");
    Json(session_state(&state, method.map(|Extension(m)| m)))
}

pub(crate) async fn shell_snapshot_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
) -> Response {
    debug!(user_id = %identity.user_id, "T3 UI shell snapshot bootstrap");
    match shell_snapshot(
        &state.reads,
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
/// A 404 here is not an error to the client — it defers to the socket
/// subscription, and is also what an unreadable conversation gets, so a 403
/// wouldn't leak existence.
pub(crate) async fn thread_snapshot_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path(thread_id): Path<String>,
) -> Response {
    debug!(user_id = %identity.user_id, task_id = %thread_id, "T3 UI thread snapshot bootstrap");
    match thread_snapshot(
        &state.reads,
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
pub(crate) async fn scripts_list_http(
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

/// Start a Task's declared Script from outside the RPC/WS surface (`moat tasks
/// scripts run`). Same effect as `scripts.run`: execs into a named terminal
/// session, publishing the Script's port first if it declares one.
pub(crate) async fn scripts_run_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, script_id)): Path<(String, String)>,
) -> Response {
    let viewer = viewer_scope(&identity);
    let payload = json!({ "threadId": thread_id, "scriptId": script_id });
    match scripts::run(
        &state.scripts,
        state.reads.conversations(),
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

pub(crate) async fn websocket_upgrade(
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
/// Only the cookie case needs it — a cookie is auto-attached by the browser,
/// unlike a ticket's bearer token; an unknown method defaults to the safer
/// cookie case.
pub(crate) fn origin_checked(method: Option<Extension<UiRpcAuthMethod>>) -> bool {
    !matches!(method, Some(Extension(UiRpcAuthMethod::WebSocketTicket)))
}

pub(crate) fn origin_allowed(headers: &HeaderMap, allowed_origins: &[AllowedOrigin]) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            allowed_origins
                .iter()
                .any(|allowed| allowed.matches(origin))
        })
}

/// Capacity and PostgreSQL statement cancellation are transient overload, not
/// malformed requests or permanent server defects. HTTP clients may retry them;
/// every other database error remains an internal failure.
pub(crate) fn shell_failure_status(error: &sqlx::Error) -> StatusCode {
    match error {
        sqlx::Error::PoolTimedOut => StatusCode::SERVICE_UNAVAILABLE,
        sqlx::Error::Database(error) if error.code().as_deref() == Some("57014") => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// How a failed `scripts.run`/list reaches an HTTP caller (`moat tasks
/// scripts`) — the same refusal vocabulary as [`scripts_failure_exit`], the WS
/// twin, as status codes instead of RPC exit shapes.
pub(crate) fn scripts_error_response(error: &ScriptsError) -> Response {
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
            warn!(
                ?terminal_error,
                "scripts.run's terminal could not be started"
            );
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
