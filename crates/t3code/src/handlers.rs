use crate::authentication::viewer_scope;
use crate::config::ORCHESTRATION_PROTOCOL_HEADER;
use crate::config::ORCHESTRATION_PROTOCOL_QUERY_PARAM;
use crate::config::OrchestrationProtocol;
use crate::oauth::invalid_request;
use crate::projection;
use crate::projection::Compactions;
use crate::projection::HistoryCursor;
use crate::pull_request_conversation;
use crate::pull_request_review;
use crate::rpc::config::descriptor;
use crate::rpc::config::session_state;
use crate::rpc::errors::pull_request_review_error;
use crate::rpc::socket::MAX_FRAME_BYTES;
use crate::rpc::socket::serve_websocket;
use crate::scripts;
use crate::scripts::ScriptsError;
use crate::snapshots::SnapshotShape;
use crate::snapshots::ThreadDetailV2;
use crate::snapshots::shell_snapshot_in;
use crate::snapshots::thread_detail_v2;
use crate::snapshots::thread_snapshot;
use crate::state::UiRpcState;
use axum::Json;
use axum::extract::Extension;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::RawQuery;
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
use serde::Deserialize;
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
    headers: HeaderMap,
) -> Response {
    debug!(user_id = %identity.user_id, "T3 UI shell snapshot bootstrap");
    match shell_snapshot_in(
        &state.reads,
        &state.model_defaults,
        &viewer_scope(&identity),
        header_protocol(&state, &headers),
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
    headers: HeaderMap,
) -> Response {
    debug!(user_id = %identity.user_id, task_id = %thread_id, "T3 UI thread snapshot bootstrap");
    let viewer = viewer_scope(&identity);
    let snapshot = match header_protocol(&state, &headers) {
        OrchestrationProtocol::V1 => {
            thread_snapshot(&state.reads, &state.deployment(), &viewer, &thread_id)
                .await
                .map(|snapshot| snapshot.map(|snapshot| snapshot.value))
        }
        OrchestrationProtocol::V2 => thread_detail_v2(
            &state.reads,
            &state.deployment(),
            state.subtasks.relations.as_ref(),
            &viewer,
            &thread_id,
        )
        .await
        .map(|detail| detail.map(|detail| detail.snapshot)),
    };
    match snapshot {
        Ok(Some(snapshot)) => Json(snapshot).into_response(),
        Ok(None) => conversation_not_found(),
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

/// The contract's `threadBoundedSnapshot` query: each opt-in is on at `"1"`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoundedSnapshotQuery {
    compact_turn_items: Option<String>,
    dedupe_bodies: Option<String>,
    compact_ids: Option<String>,
}

/// The contract's `threadHistoryPage` query.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryPageQuery {
    cursor: Option<String>,
    through_entry_id: Option<String>,
    view: Option<String>,
    compact_ids: Option<String>,
}

fn opted_in(flag: Option<&String>) -> bool {
    flag.is_some_and(|flag| flag == "1")
}

/// The newest window of a thread, for a client that pages older history in
/// with [`thread_history_http`]. Absences answer as [`thread_snapshot_http`]'s
/// do, so a client that gets one falls back to the whole snapshot.
pub(crate) async fn thread_bounded_snapshot_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path(thread_id): Path<String>,
    Query(query): Query<BoundedSnapshotQuery>,
    headers: HeaderMap,
) -> Response {
    let mut detail = match readable_detail_v2(&state, &identity, &thread_id, &headers).await {
        Ok(detail) => detail,
        Err(response) => return response,
    };
    let shape = SnapshotShape {
        bounded: true,
        compactions: Compactions {
            dedupe_bodies: opted_in(query.dedupe_bodies.as_ref()),
            compact_turn_items: opted_in(query.compact_turn_items.as_ref()),
            compact_ids: opted_in(query.compact_ids.as_ref()),
        },
    };
    Json(detail.take_shaped(json!({}), shape)).into_response()
}

/// The rows older than a cursor [`thread_bounded_snapshot_http`] or an
/// earlier page handed out, cut from the same whole projection.
pub(crate) async fn thread_history_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path(thread_id): Path<String>,
    Query(query): Query<HistoryPageQuery>,
    headers: HeaderMap,
) -> Response {
    let detail = match readable_detail_v2(&state, &identity, &thread_id, &headers).await {
        Ok(detail) => detail,
        Err(response) => return response,
    };
    let Some(cursor) = query.cursor.as_deref().and_then(HistoryCursor::decode) else {
        return invalid_request("invalid_history_cursor");
    };
    let mut page = projection::history_page(
        &detail.snapshot["projection"],
        detail.snapshot["snapshotSequence"].as_u64().unwrap_or(0),
        &cursor,
        query.through_entry_id.as_deref(),
        query.view.as_deref() == Some("conversation"),
    );
    if opted_in(query.compact_ids.as_ref()) {
        projection::with_id_table(&mut page, None);
    }
    Json(page).into_response()
}

/// One readable thread's whole V2 projection, or the response that says it
/// is absent: a V1 request names no route here, so it is absent too.
async fn readable_detail_v2(
    state: &UiRpcState,
    identity: &UiRpcAuthIdentity,
    thread_id: &str,
    headers: &HeaderMap,
) -> Result<ThreadDetailV2, Response> {
    debug!(user_id = %identity.user_id, task_id = %thread_id, "T3 UI bounded thread read");
    if header_protocol(state, headers) != OrchestrationProtocol::V2 {
        return Err(conversation_not_found());
    }
    let detail = thread_detail_v2(
        &state.reads,
        &state.deployment(),
        state.subtasks.relations.as_ref(),
        &viewer_scope(identity),
        thread_id,
    )
    .await;
    match detail {
        Ok(Some(detail)) => Ok(detail),
        Ok(None) => Err(conversation_not_found()),
        Err(error) => {
            error!(
                %error,
                user_id = %identity.user_id,
                task_id = %thread_id,
                "Failed to assemble T3 UI thread snapshot"
            );
            Err((
                shell_failure_status(&error),
                Json(json!({ "error": "Failed to assemble the conversation" })),
            )
                .into_response())
        }
    }
}

fn conversation_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "Conversation not found" })),
    )
        .into_response()
}

/// A Task's Scripts, for a caller outside the RPC/WS surface (`moat tasks
/// scripts list`) — the same read `scripts.run` resolves a Script id against,
/// so it carries the Task's own Scripts as well as its Workspace's.
pub(crate) async fn scripts_list_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path(thread_id): Path<String>,
) -> Response {
    let viewer = viewer_scope(&identity);
    match scripts::list(
        &state.scripts,
        state.sandboxes.as_ref(),
        &viewer,
        &thread_id,
    )
    .await
    {
        Ok(scripts) => {
            Json(json!({ "scripts": scripts::public_script_list(&scripts) })).into_response()
        }
        Err(error) => scripts_error_response(&error),
    }
}

/// Declare a Script on this Task alone (`moat tasks scripts run --command`).
///
/// Nothing is written to the Workspace, so no other Task under it gains a
/// control; the record lives in this Task's sandbox.
pub(crate) async fn scripts_declare_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, script_id)): Path<(String, String)>,
    body: String,
) -> Response {
    let viewer = viewer_scope(&identity);
    let payload = match request_payload(&body) {
        Ok(payload) => payload,
        Err(detail) => return scripts_error_response(&ScriptsError::Invalid(detail)),
    };
    match scripts::declare(
        &state.scripts,
        state.sandboxes.as_ref(),
        &viewer,
        &thread_id,
        &script_id,
        &payload,
    )
    .await
    {
        Ok(script) => Json(json!({ "script": script.name })).into_response(),
        Err(error) => scripts_error_response(&error),
    }
}

/// Drop a Script this Task declared on itself; a Workspace Script of the same
/// name is untouched and becomes visible again.
pub(crate) async fn scripts_remove_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, script_id)): Path<(String, String)>,
) -> Response {
    let viewer = viewer_scope(&identity);
    match scripts::remove(
        &state.scripts,
        state.sandboxes.as_ref(),
        &viewer,
        &thread_id,
        &script_id,
    )
    .await
    {
        Ok(()) => Json(json!({ "removed": script_id })).into_response(),
        Err(error) => scripts_error_response(&error),
    }
}

/// Start a Task's Script from outside the RPC/WS surface (`moat tasks scripts
/// run`). Same effect as `scripts.run`: execs into a named terminal session,
/// publishing the Script's port first if it declares one.
///
/// A body carrying `command` declares the Script on this Task first, which is
/// what makes an ad-hoc server one call rather than two.
pub(crate) async fn scripts_run_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    Path((thread_id, script_id)): Path<(String, String)>,
    body: String,
) -> Response {
    let viewer = viewer_scope(&identity);
    let declaration = match request_payload(&body) {
        Ok(payload) => payload,
        Err(detail) => return scripts_error_response(&ScriptsError::Invalid(detail)),
    };
    if declaration.get("command").is_some() {
        if let Err(error) = scripts::declare(
            &state.scripts,
            state.sandboxes.as_ref(),
            &viewer,
            &thread_id,
            &script_id,
            &declaration,
        )
        .await
        {
            return scripts_error_response(&error);
        }
    }

    let input = scripts::ScriptsRunInput {
        thread_id,
        script_id,
    };
    match scripts::run(
        &state.scripts,
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &input,
    )
    .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => scripts_error_response(&error),
    }
}

/// `POST /api/pull-requests/diff`: one page of a linked pull request's change,
/// read from its git host. A failure is T3's `PullRequestOperationError`, sent
/// with the 502 its schema declares.
pub(crate) async fn pull_request_diff_http(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    body: String,
) -> Response {
    const OPERATION: &str = "pullRequests.diff";
    let refused = |detail: &str| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "_tag": "PullRequestOperationError",
                "operation": OPERATION,
                "detail": detail,
            })),
        )
            .into_response()
    };
    let payload = match request_payload(&body) {
        Ok(payload) => payload,
        Err(detail) => return refused(&detail),
    };
    let Some((project_id, repository, number)) = pull_request_review::pull_request_ref(&payload)
    else {
        return refused("A pull request reference needs a projectId, repository and number.");
    };
    let commit = payload
        .get("commit")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|commit| !commit.is_empty());
    if commit.is_some_and(|commit| !pull_request_conversation::is_commit_sha(commit)) {
        return refused("A commit is named by its id.");
    }
    let target = pull_request_review::PullRequestTarget {
        project_id: &project_id,
        repository: &repository,
        number,
    };
    let cursor = payload.get("cursor").and_then(Value::as_str);
    match pull_request_review::diff(
        &pull_request_review::review_ports(&state),
        &viewer_scope(&identity),
        &target,
        commit,
        cursor,
    )
    .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(pull_request_review_error(OPERATION, &error, &identity)),
        )
            .into_response(),
    }
}

/// The request body as an object, or why it is not one. An empty body is an
/// empty object, since `moat tasks scripts run` sends no fields when it is only
/// starting a Script.
///
/// Returns the detail rather than a built `Response`: an `Err` holding one
/// trips `clippy::result_large_err`.
fn request_payload(body: &str) -> Result<Value, String> {
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(body).map_err(|error| format!("Body is not JSON: {error}"))
}

pub(crate) async fn websocket_upgrade(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
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

    let protocol = state
        .rpc
        .protocol_for(requested_protocol(query.as_deref()).as_deref());
    websocket
        .max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| serve_websocket(socket, state, identity, protocol))
}

/// The protocol an HTTP snapshot read is answered in.
fn header_protocol(state: &UiRpcState, headers: &HeaderMap) -> OrchestrationProtocol {
    state.rpc.protocol_for(
        headers
            .get(ORCHESTRATION_PROTOCOL_HEADER)
            .and_then(|value| value.to_str().ok()),
    )
}

/// The protocol version a `/ws` query string asks for, if it names one.
pub(crate) fn requested_protocol(query: Option<&str>) -> Option<String> {
    // Unescaped: the only value a client sends is a bare version number.
    query?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == ORCHESTRATION_PROTOCOL_QUERY_PARAM)
        .map(|(_, value)| value.to_string())
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
        ScriptsError::Invalid(detail) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": detail.clone() })),
        )
            .into_response(),
        // The same `code` a port refusal carries: reaching the Task's own
        // Scripts needs the sandbox running, and starting it is the fix.
        // `NotFound` is that state too — a pod that has not registered this Task
        // yet answers it — so it is a refusal to retry, not a defect.
        ScriptsError::Sandbox(SandboxError::NotReady(_) | SandboxError::NotFound(_)) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Sandbox not running.", "code": "SANDBOX_NOT_RUNNING" })),
        )
            .into_response(),
        ScriptsError::Sandbox(SandboxError::BadRequest(detail)) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": detail.clone() })),
        )
            .into_response(),
        ScriptsError::Sandbox(cause) => {
            warn!(%cause, "A task's own scripts could not be reached");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "The task's own scripts could not be reached." })),
            )
                .into_response()
        }
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
