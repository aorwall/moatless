//! Sharing & access: the pairing links and paired devices T3's Authorized
//! clients section lists and revokes.
//!
//! A pairing link is an unredeemed pairing code and a client session is a
//! paired device's API key, both read through [`DevicePairing`]. Only a browser
//! session may change either: see [`browser_session_only`].

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use moatless_identity::auth::pairing::{DevicePairing, PairedDeviceKey, PairingCode};
use moatless_types::ui_rpc::{UiRpcAuthIdentity, UiRpcAuthMethod};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, warn};

use crate::oauth::{GRANTED_SCOPES, Scope, error_body, internal_error, invalid_request};
use crate::state::UiRpcState;
use crate::subscription::{StreamFailure, StreamResult, StreamSink};

/// How often an open Authorized clients list re-reads without a nudge. The
/// nudge from [`AccessChanges`] reaches only this replica, so this bounds how
/// stale a change made through another one shows.
const RESNAPSHOT_INTERVAL: Duration = Duration::from_secs(5);

/// Nudges this replica's open access subscriptions to re-read.
#[derive(Clone)]
pub(crate) struct AccessChanges(Arc<watch::Sender<u64>>);

impl AccessChanges {
    pub(crate) fn new() -> Self {
        Self(Arc::new(watch::Sender::new(0)))
    }

    pub(crate) fn notify(&self) {
        self.0
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

fn instant(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// What every link and device is reported to hold: the deployment's grant, the
/// one the socket enforces whatever a pairing asked for.
fn granted_scopes() -> Vec<&'static str> {
    GRANTED_SCOPES.iter().map(|scope| scope.as_str()).collect()
}

fn pairing_link(code: &PairingCode) -> Value {
    let mut link = json!({
        "id": code.id,
        "scopes": granted_scopes(),
        "subject": code.user_id,
        "createdAt": instant(code.created_at),
        "expiresAt": instant(code.expires_at),
    });
    if let Some(label) = &code.label {
        link["label"] = json!(label);
    }
    link
}

/// The OS a key's name ends in, as `key_name` writes it: `Label (OS)`.
fn os_of(name: &str) -> Option<&str> {
    let inner = name.strip_suffix(')')?;
    let (_, os) = inner.rsplit_once(" (")?;
    let os = os.trim();
    (!os.is_empty()).then_some(os)
}

fn client_session(user_id: &str, device: &PairedDeviceKey) -> Value {
    let os = os_of(&device.name);
    let device_type = match os.map(str::to_ascii_lowercase).as_deref() {
        Some("ios" | "android") => "mobile",
        Some("ipados") => "tablet",
        Some("macos" | "windows" | "linux") => "desktop",
        _ => "unknown",
    };
    let mut client = json!({ "label": device.name, "deviceType": device_type });
    if let Some(os) = os {
        client["os"] = json!(os);
    }
    json!({
        "sessionId": device.id,
        "subject": user_id,
        "scopes": granted_scopes(),
        "method": "bearer-access-token",
        "client": client,
        "issuedAt": instant(device.created_at),
        "expiresAt": instant(device.expires_at),
        "lastConnectedAt": device.last_used_at.map(instant),
        // Neither is known here: a socket does not record the key behind its
        // ticket, so no listed device can be told apart as live or as this one.
        "connected": false,
        "current": false,
    })
}

/// The `AuthAccessSnapshot` for `user_id`.
pub(crate) async fn snapshot(pairing: &DevicePairing, user_id: &str) -> Result<Value, String> {
    let codes = pairing
        .pending_codes(user_id)
        .await
        .map_err(|error| error.to_string())?;
    let devices = pairing
        .devices(user_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "pairingLinks": codes.iter().map(pairing_link).collect::<Vec<_>>(),
        "clientSessions": devices
            .iter()
            .map(|device| client_session(user_id, device))
            .collect::<Vec<_>>(),
    }))
}

/// `subscribeAuthAccess`: a snapshot now, and another whenever it changes.
pub(crate) async fn watch_access(
    sink: StreamSink,
    pairing: Arc<DevicePairing>,
    changes: AccessChanges,
    user_id: String,
) -> StreamResult {
    let mut nudges = changes.subscribe();
    let mut ticks = interval(RESNAPSHOT_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticks.tick().await;
    let mut sent: Option<Value> = None;
    let mut revision = 0_u64;
    loop {
        match snapshot(&pairing, &user_id).await {
            Ok(payload) if sent.as_ref() != Some(&payload) => {
                revision += 1;
                let event = json!({
                    "version": 1,
                    "revision": revision,
                    "type": "snapshot",
                    "payload": payload,
                });
                if !sink.send(event).await {
                    return Ok(());
                }
                sent = Some(payload);
            }
            Ok(_) => {}
            // The first read failing leaves the client nothing to show, so it
            // ends the stream; a later one keeps the last snapshot standing.
            Err(error) if sent.is_none() => {
                error!(%error, %user_id, "Failed to read T3 authorized clients");
                return Err(StreamFailure::message("client_sessions_load_failed"));
            }
            Err(error) => warn!(%error, %user_id, "Failed to re-read T3 authorized clients"),
        }
        tokio::select! {
            _ = ticks.tick() => {}
            changed = nudges.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

/// T3's `insufficient_scope` refusal for every caller but a browser session.
///
/// A paired device holds an API key, and a credential that can mint or revoke
/// credentials must not be one a device can be given; this is the same rule as
/// `create_pairing_code_handler`'s.
fn browser_session_only(method: Option<UiRpcAuthMethod>) -> Option<Response> {
    if method == Some(UiRpcAuthMethod::SessionCookie) {
        return None;
    }
    let mut body = error_body("EnvironmentScopeRequiredError", "insufficient_scope", "");
    if let Some(body) = body.as_object_mut() {
        body.remove("reason");
        body.insert("requiredScope".into(), json!(Scope::AccessWrite.as_str()));
    }
    Some((StatusCode::FORBIDDEN, Json(body)).into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreatePairingCredential {
    label: Option<String>,
    scopes: Option<Vec<String>>,
}

/// `POST /api/auth/pairing-token`: mint a pairing code, as a link.
pub(crate) async fn create_pairing_credential(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
    body: Result<Json<CreatePairingCredential>, JsonRejection>,
) -> Response {
    if let Some(refused) = browser_session_only(method.map(|Extension(m)| m)) {
        return refused;
    }
    let Ok(Json(body)) = body else {
        return invalid_request("invalid_command");
    };
    // A device gets the whole grant whatever it asks for, so a narrower or a
    // wider request is refused rather than recorded as a promise not kept.
    if let Some(requested) = &body.scopes {
        let granted = granted_scopes();
        let same = requested.len() == granted.len()
            && granted
                .iter()
                .all(|scope| requested.iter().any(|r| r == scope));
        if !same {
            warn!(?requested, user_id = %identity.user_id, "Refused a T3 pairing link narrower or wider than the grant");
            return invalid_request("invalid_scope");
        }
    }
    let label = body
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty());
    match state
        .pairing
        .issue_code(&identity.user_id.to_string(), label)
        .await
    {
        Ok(issued) => {
            state.access_changes.notify();
            let mut result = json!({
                "id": issued.id,
                "credential": issued.code,
                "expiresAt": instant(issued.expires_at),
            });
            if let Some(label) = issued.label {
                result["label"] = json!(label);
            }
            Json(result).into_response()
        }
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "Failed to issue a T3 pairing link");
            internal_error("pairing_credential_issuance_failed")
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RevokePairingLink {
    id: String,
}

/// `POST /api/auth/pairing-links/revoke`.
pub(crate) async fn revoke_pairing_link(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
    body: Result<Json<RevokePairingLink>, JsonRejection>,
) -> Response {
    if let Some(refused) = browser_session_only(method.map(|Extension(m)| m)) {
        return refused;
    }
    let Ok(Json(body)) = body else {
        return invalid_request("invalid_command");
    };
    match state
        .pairing
        .revoke_code(&identity.user_id.to_string(), &body.id)
        .await
    {
        Ok(revoked) => {
            state.access_changes.notify();
            Json(json!({ "revoked": revoked })).into_response()
        }
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "Failed to revoke a T3 pairing link");
            internal_error("pairing_link_revoke_failed")
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevokeClient {
    session_id: String,
}

/// `POST /api/auth/clients/revoke`: unpair one device.
pub(crate) async fn revoke_client(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
    body: Result<Json<RevokeClient>, JsonRejection>,
) -> Response {
    if let Some(refused) = browser_session_only(method.map(|Extension(m)| m)) {
        return refused;
    }
    let Ok(Json(body)) = body else {
        return invalid_request("invalid_command");
    };
    match state
        .pairing
        .revoke_device(&identity.user_id.to_string(), &body.session_id)
        .await
    {
        Ok(revoked) => {
            state.access_changes.notify();
            Json(json!({ "revoked": revoked })).into_response()
        }
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "Failed to unpair a T3 client");
            internal_error("client_session_revoke_failed")
        }
    }
}

/// `POST /api/auth/clients/revoke-others`: unpair every device. The caller is
/// a browser session, which is not among them.
pub(crate) async fn revoke_other_clients(
    State(state): State<UiRpcState>,
    Extension(identity): Extension<UiRpcAuthIdentity>,
    method: Option<Extension<UiRpcAuthMethod>>,
) -> Response {
    if let Some(refused) = browser_session_only(method.map(|Extension(m)| m)) {
        return refused;
    }
    match state
        .pairing
        .revoke_all_devices(&identity.user_id.to_string())
        .await
    {
        Ok(count) => {
            state.access_changes.notify();
            Json(json!({ "revokedCount": count })).into_response()
        }
        Err(error) => {
            error!(%error, user_id = %identity.user_id, "Failed to unpair T3 clients");
            internal_error("client_session_revoke_failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_name_yields_its_os() {
        assert_eq!(os_of("T3 Code Mobile (iOS)"), Some("iOS"));
        assert_eq!(os_of("Bedside (phone) (Android)"), Some("Android"));
        assert_eq!(os_of("Paired device"), None);
        assert_eq!(os_of("Odd ()"), None);
    }
}
