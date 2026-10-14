//! Methods that answer for the deployment itself.

use super::routing::Request;
use crate::authentication::viewer_scope;
use crate::config::T3_CLIENT_CONTRACT_VERSION;
use crate::git_actions;
use crate::providers;
use crate::rpc::config::now_rfc3339;
use crate::rpc::config::platform_arch;
use crate::rpc::config::platform_os;
use crate::rpc::config::server_config;
use crate::rpc::envelope::success_exit;
use crate::rpc::envelope::typed_failure_exit;
use crate::rpc::session::resolve_workspace_skills;
use crate::usage;
use serde_json::Value;
use serde_json::json;
use tracing::warn;

pub(super) fn server_probe(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!({}))]
}

pub(super) fn server_get_settings(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, json!({}))]
}

/// Accepted and dropped, never refused.
///
/// Moatless runs agents in Sandboxes no browser tab is attached to, so this
/// per-focus-change report is meaningless here, and refusing it would just be
/// noise.
pub(super) fn server_report_client_activity(request: Request) -> Vec<Value> {
    vec![success_exit(&request.id, Value::Null)]
}

pub(super) fn cloud_get_relay_client_status(request: Request) -> Vec<Value> {
    vec![success_exit(
        &request.id,
        json!({
            "status": "unsupported",
            "platform": platform_os(),
            "arch": platform_arch(),
            "version": T3_CLIENT_CONTRACT_VERSION
        }),
    )]
}

pub(super) async fn server_get_config(request: Request) -> Vec<Value> {
    let Request {
        id,
        state,
        identity,
        shared,
        ..
    } = request;
    let snapshots = shared.workspace_snapshots.published();
    vec![success_exit(
        &id,
        server_config(&state, &identity, &snapshots).await,
    )]
}

/// Publishes a fresh provider list twice: as the reply, which is what stops the
/// composer asking again, and on the open config stream, which is what fills
/// the menu.
pub(super) async fn server_refresh_providers(request: Request) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        shared,
        ..
    } = request;
    if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
        resolve_workspace_skills(&state, &identity, cwd, &shared.workspace_snapshots).await;
    }
    let agents = state.agents.list(&viewer_scope(&identity)).await;
    let providers = providers::providers(
        &agents,
        &state.model_defaults,
        &now_rfc3339(),
        &shared.workspace_snapshots.published(),
    );
    let mut frames = vec![success_exit(&id, json!({ "providers": providers.clone() }))];
    if let Some(stream) = shared.config_stream.opened() {
        frames.push(json!({
            "_tag": "Chunk",
            "requestId": stream.wire(),
            "values": [{
                "version": 1,
                "type": "providerStatuses",
                "payload": { "providers": providers }
            }]
        }));
    }
    frames
}

pub(super) async fn server_get_usage_summary(request: Request) -> Vec<Value> {
    let Request {
        id,
        tag,
        payload,
        state,
        identity,
        ..
    } = request;
    let window = match usage::window_from(&payload) {
        Ok(window) => window,
        Err(detail) => {
            return vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "UsageReadError",
                    "reason": "invalidWindow",
                    "detail": detail,
                }),
            )];
        }
    };
    let started = std::time::Instant::now();
    match state.usage.buckets(&viewer_scope(&identity), &window).await {
        Ok(rows) => {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            vec![success_exit(
                &id,
                usage::summary(&window, &rows, &now_rfc3339(), elapsed),
            )]
        }
        // The cause is logged, not sent: it names tables and columns, and the
        // client renders `detail` verbatim.
        Err(cause) => {
            warn!(%tag, user_id = %identity.user_id, %cause, "T3 usage read failed");
            vec![typed_failure_exit(
                &id,
                json!({
                    "_tag": "UsageReadError",
                    "reason": "scanFailed",
                    "detail": "This deployment could not read usage.",
                }),
            )]
        }
    }
}

pub(super) async fn server_discover_source_control(request: Request) -> Vec<Value> {
    let Request {
        id,
        state,
        identity,
        ..
    } = request;
    let github = state
        .source_control_auth
        .github_identity(&viewer_scope(&identity))
        .await;
    vec![success_exit(&id, git_actions::discovery(github.as_ref()))]
}
