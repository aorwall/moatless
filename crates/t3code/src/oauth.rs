//! How a T3 Code client on another device comes to hold a credential.
//!
//! `POST /oauth/token` trades a pairing code for the token a device keeps;
//! `POST /api/auth/websocket-ticket` trades that token for a ticket that
//! survives the one hop the token cannot: a browser-grade `WebSocket`
//! constructor, which sends no headers at all. Both are translation only —
//! redeemed through the [`crate::DevicePairing`] port into an ordinary Moatless
//! API key, verified by the root backend's own auth middleware.

use axum::Json;
use axum::extract::rejection::FormRejection;
use axum::extract::{Extension, Form, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{error, info, warn};

use super::UiRpcState;
use crate::auth::{WS_TICKET_TTL_SECS, generate_ws_ticket_jwt};
use moatless_identity::auth::pairing::{PairingClient, PairingError};
use moatless_types::ui_rpc::UiRpcAuthIdentity;

/// The scopes this deployment is able to grant, in the order it reports them.
///
/// Checked against [`required_scope`] before dispatch; `access:write` and
/// `relay:write` stay out because nothing behind this deployment needs them.
pub(super) const GRANTED_SCOPES: [&str; 6] = [
    "orchestration:read",
    "orchestration:operate",
    "terminal:operate",
    "review:write",
    "access:read",
    "relay:read",
];

/// Every scope name T3's contract defines, granted here or not.
///
/// Kept apart from [`GRANTED_SCOPES`]: asking for an ungranted scope and asking
/// for a nonexistent one are different mistakes, answered differently.
const KNOWN_SCOPES: [&str; 8] = [
    "orchestration:read",
    "orchestration:operate",
    "terminal:operate",
    "review:write",
    "access:read",
    "access:write",
    "relay:read",
    "relay:write",
];

/// The scope a method requires, ported from the reference server's
/// `RPC_REQUIRED_SCOPES` (`apps/server/src/auth/RpcAuthorization.ts:23`).
///
/// Runs ahead of dispatch, listing every method the *contract* declares, so a
/// withheld surface is refused for its scope rather than told it doesn't exist;
/// the `orchestration:read` default gives an unclassified method the least
/// authority.
pub(super) fn required_scope(tag: &str) -> &'static str {
    match tag {
        // Writes.
        "orchestration.dispatchCommand"
        | "server.refreshProviders"
        | "server.updateProvider"
        | "server.updateServer"
        | "server.updateServerWithProgress"
        | "server.upsertKeybinding"
        | "server.removeKeybinding"
        | "server.updateSettings"
        | "server.retryResourceTelemetry"
        | "server.signalProcess"
        | "server.reportHostPowerState"
        | "sourceControl.cloneRepository"
        | "sourceControl.publishRepository"
        | "projects.add"
        | "projects.remove"
        | "projects.writeFile"
        | "shell.openInEditor"
        | "vcs.pull"
        | "vcs.createWorktree"
        | "vcs.removeWorktree"
        | "vcs.createRef"
        | "vcs.switchRef"
        | "vcs.init"
        | "git.runStackedAction"
        | "git.resolvePullRequest"
        | "git.preparePullRequestThread"
        | "preview.open"
        | "preview.navigate"
        | "preview.resize"
        | "preview.refresh"
        | "preview.close"
        | "preview.reportStatus"
        | "preview.automationConnect"
        | "preview.automationRespond"
        | "preview.automationFocusHost"
        | "attachments.createUploadUrl"
        | "attachments.delete"
        | "sandbox.restart"
        | "sandbox.redeploy"
        | "sandbox.cleanup"
        | "sandbox.setIdleTimeout" => "orchestration:operate",

        // The diff surface. `review:write` is what gates it in the reference
        // server's map, despite the name — this backend serves it under that
        // scope.
        "review.getDiffPreview" => "review:write",

        // Terminals. The backend authorizes them here; the PTY itself lives in
        // the conversation's Sandbox.
        "terminal.open"
        | "terminal.attach"
        | "terminal.write"
        | "terminal.resize"
        | "terminal.clear"
        | "terminal.restart"
        | "terminal.close"
        | "subscribeTerminalEvents"
        | "subscribeTerminalMetadata" => "terminal:operate",

        "cloud.getRelayClientStatus" => "relay:read",
        "cloud.installRelayClient" => "relay:write",
        "subscribeAuthAccess" => "access:read",

        _ => "orchestration:read",
    }
}

const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const BOOTSTRAP_TOKEN_TYPE: &str = "urn:t3:params:oauth:token-type:environment-bootstrap";
const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";

/// RFC 8693 token exchange, as T3 sends it: form-encoded, not JSON.
#[derive(Debug, Deserialize)]
pub(super) struct TokenExchangeRequest {
    grant_type: String,
    /// The pairing code, which the client calls a subject token.
    subject_token: String,
    subject_token_type: String,
    requested_token_type: String,
    scope: Option<String>,
    /// What the client calls itself. Unverified, and used only to name the key.
    client_label: Option<String>,
    client_os: Option<String>,
}

/// Trade a pairing code for the token a device keeps.
///
/// The one route in this module not authenticated by
/// [`crate::oauth::middleware`] — exempted because the credential it needs is
/// in the body itself.
pub(super) async fn token_exchange(
    State(state): State<UiRpcState>,
    request: Result<Form<TokenExchangeRequest>, FormRejection>,
) -> Response {
    let Ok(Form(request)) = request else {
        // A body this backend could not read at all. The contract has no reason
        // for it, and `invalid_command` is the nearest true one: the request was
        // not the request this endpoint takes.
        return invalid_request("invalid_command");
    };

    if request.grant_type != TOKEN_EXCHANGE_GRANT_TYPE
        || request.subject_token_type != BOOTSTRAP_TOKEN_TYPE
        || request.requested_token_type != ACCESS_TOKEN_TYPE
    {
        warn!(
            grant_type = %request.grant_type,
            subject_token_type = %request.subject_token_type,
            requested_token_type = %request.requested_token_type,
            "Rejected a T3 token exchange asking for a grant this environment does not offer"
        );
        return invalid_request("invalid_command");
    }

    let scopes = match granted_scopes(request.scope.as_deref()) {
        Ok(scopes) => scopes,
        Err(reason) => {
            warn!(scope = ?request.scope, reason, "Rejected a T3 token exchange over scope");
            return invalid_request(reason);
        }
    };

    let client = PairingClient {
        label: request.client_label,
        os: request.client_os,
    };
    match state
        .pairing
        .redeem_code(&request.subject_token, &client, scopes.clone())
        .await
    {
        Ok(device) => {
            info!(key_id = %device.key_id, "Paired a T3 client");
            Json(json!({
                "access_token": device.token,
                "issued_token_type": ACCESS_TOKEN_TYPE,
                "token_type": "Bearer",
                "expires_in": (device.expires_at - Utc::now()).num_seconds().max(0),
                "scope": scopes.join(" ")
            }))
            .into_response()
        }
        Err(PairingError::InvalidCode) => {
            warn!("Rejected a T3 token exchange presenting a code that cannot be redeemed");
            auth_invalid()
        }
        Err(PairingError::Storage(error)) => {
            error!(%error, "Failed to redeem a T3 pairing code");
            internal_error("access_token_issuance_failed")
        }
    }
}

/// Trade the token a device holds for a ticket that can be spent on the socket.
///
/// Middleware already authenticated the client via its `Authorization` header;
/// this reads who that was and signs it.
pub(super) async fn websocket_ticket(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
) -> Response {
    let Some(secret) = state.session_jwt_secret.as_deref() else {
        error!("Cannot issue a T3 WebSocket ticket: no JWT secret is configured");
        return internal_error("websocket_ticket_issuance_failed");
    };

    match generate_ws_ticket_jwt(&identity, secret) {
        Ok(ticket) => {
            let expires_at = Utc::now() + chrono::Duration::seconds(WS_TICKET_TTL_SECS as i64);
            Json(json!({
                "ticket": ticket,
                "expiresAt": expires_at.to_rfc3339_opts(SecondsFormat::Millis, true)
            }))
            .into_response()
        }
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "Failed to sign a T3 WebSocket ticket");
            internal_error("websocket_ticket_issuance_failed")
        }
    }
}

/// What a client asking for `requested` is given.
///
/// The overlap of `requested` and [`GRANTED_SCOPES`], never the bare request —
/// the client stores whatever it's granted — but an empty overlap is refused.
fn granted_scopes(requested: Option<&str>) -> Result<Vec<String>, &'static str> {
    let Some(requested) = requested else {
        return Ok(GRANTED_SCOPES.iter().map(ToString::to_string).collect());
    };
    if requested
        .split(' ')
        .any(|scope| !KNOWN_SCOPES.contains(&scope))
    {
        return Err("invalid_scope");
    }
    let granted: Vec<String> = GRANTED_SCOPES
        .iter()
        .filter(|scope| requested.split(' ').any(|asked| asked == **scope))
        .map(ToString::to_string)
        .collect();
    if granted.is_empty() {
        return Err("scope_not_granted");
    }
    Ok(granted)
}

/// A T3 error body. Every failure on this surface is one of these, because the
/// client decodes the failure shapes its contract declares and treats anything
/// else as the environment being unreachable.
fn error_body(tag: &str, code: &str, reason: &str) -> Value {
    json!({
        "_tag": tag,
        "code": code,
        "reason": reason,
        // Required to be a non-empty string, and this backend has no trace to
        // hand out on these routes. The request id would be the honest value if
        // one reached here.
        "traceId": uuid::Uuid::new_v4().to_string()
    })
}

fn invalid_request(reason: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(error_body(
            "EnvironmentRequestInvalidError",
            "invalid_request",
            reason,
        )),
    )
        .into_response()
}

fn auth_invalid() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(error_body(
            "EnvironmentAuthInvalidError",
            "auth_invalid",
            "invalid_credential",
        )),
    )
        .into_response()
}

fn internal_error(reason: &'static str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(error_body(
            "EnvironmentInternalError",
            "internal_error",
            reason,
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_asking_for_nothing_is_given_everything() {
        assert_eq!(granted_scopes(None).unwrap(), GRANTED_SCOPES);
    }

    #[test]
    fn a_client_is_given_the_part_of_its_request_this_deployment_has() {
        // Verbatim what the T3 mobile client asks for.
        let granted = granted_scopes(Some(
            "orchestration:read orchestration:operate terminal:operate review:write relay:read",
        ))
        .unwrap();
        assert_eq!(
            granted,
            [
                "orchestration:read",
                "orchestration:operate",
                "terminal:operate",
                "review:write",
                "relay:read"
            ]
        );
    }

    #[test]
    fn a_request_naming_only_scopes_we_lack_is_refused() {
        // Both are contract scopes this deployment withholds: it administers no
        // pairing sessions over this surface and has no relay to install.
        assert_eq!(
            granted_scopes(Some("access:write relay:write")),
            Err("scope_not_granted")
        );
    }

    /// The gate reads this map for every request, so a method landing in the
    /// wrong bucket is either a refusal nobody can work around or a permission
    /// that was never checked.
    #[test]
    fn a_method_that_changes_something_needs_more_than_read() {
        assert_eq!(
            required_scope("orchestration.subscribeShell"),
            "orchestration:read"
        );
        assert_eq!(
            required_scope("orchestration.dispatchCommand"),
            "orchestration:operate"
        );
        assert_eq!(
            required_scope("projects.writeFile"),
            "orchestration:operate"
        );
        assert_eq!(required_scope("review.getDiffPreview"), "review:write");
    }

    /// An unknown tag is one this server does not implement, and it has to
    /// reach the dispatch to be told so. Defaulting it to anything stronger
    /// than read would answer "you may not" where the truth is "there is no
    /// such method".
    #[test]
    fn a_method_this_map_does_not_name_falls_through_to_read() {
        assert_eq!(
            required_scope("something.nobodyImplemented"),
            "orchestration:read"
        );
    }

    /// The fleet diagnostics read is unclassified here on purpose — it is a
    /// read like `sandbox.status`, so the default is the
    /// right answer rather than an omission.
    #[test]
    fn fleet_diagnostics_falls_through_to_read() {
        assert_eq!(
            required_scope("sandbox.listDiagnostics"),
            "orchestration:read"
        );
    }

    #[test]
    fn a_scope_the_contract_does_not_define_is_a_different_refusal() {
        assert_eq!(
            granted_scopes(Some("orchestration:read everything:always")),
            Err("invalid_scope")
        );
    }
}
