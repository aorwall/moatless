//! Opt-in T3 Code UI compatibility routes.
//!
//! Phase one proves the browser-to-Rust transport and authentication boundary.
//! It implements the discovery/session/shell bootstrap routes and the RPCs T3
//! invokes while opening an empty environment. Moatless domain data and writes
//! intentionally remain outside this module until their projections and
//! command semantics are specified.

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::auth::identity::AuthIdentity;

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_MESSAGES_PER_FRAME: usize = 128;
const MAX_ACTIVE_STREAMS: usize = 64;

#[derive(Debug, Clone)]
struct UiRpcConfig {
    environment_id: String,
    label: String,
    server_version: String,
    allowed_origins: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UiRpcConfigError {
    #[error("Invalid T3 UI RPC configuration: {0}")]
    Invalid(String),
}

#[derive(Clone)]
struct UiRpcState {
    rpc: Arc<UiRpcConfig>,
    session_cookie_name: Arc<str>,
}

/// Build the T3 compatibility router when its environment gate is enabled.
///
/// Configuration stays at this HTTP boundary rather than becoming part of
/// sandbox or pod configuration. The global Moatless auth middleware protects
/// every route except the public discovery descriptor.
pub fn router_from_env(session_cookie_name: String) -> Result<Option<Router>, UiRpcConfigError> {
    if !env_enabled("T3_UI_RPC_ENABLED")? {
        return Ok(None);
    }

    let config = UiRpcConfig {
        environment_id: env_value("T3_UI_RPC_ENVIRONMENT_ID", "moatless"),
        label: env_value("T3_UI_RPC_LABEL", "Moatless"),
        server_version: env_value("T3_UI_RPC_SERVER_VERSION", env!("CARGO_PKG_VERSION")),
        allowed_origins: env::var("T3_UI_RPC_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(String::from)
            .collect(),
    };
    config.validate()?;
    info!(
        environment_id = %config.environment_id,
        "T3 UI RPC compatibility routes registered"
    );
    Ok(Some(router(
        Arc::new(config),
        Arc::from(session_cookie_name),
    )))
}

fn router(rpc: Arc<UiRpcConfig>, session_cookie_name: Arc<str>) -> Router {
    Router::new()
        .route("/.well-known/t3/environment", get(environment_descriptor))
        .route("/api/auth/session", get(auth_session))
        .route("/api/orchestration/shell", get(shell_snapshot_http))
        .route("/ws", get(websocket_upgrade))
        .with_state(UiRpcState {
            rpc,
            session_cookie_name,
        })
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
                "T3_UI_RPC_ALLOWED_ORIGINS must contain at least one exact origin".into(),
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

async fn shell_snapshot_http(Extension(identity): Extension<AuthIdentity>) -> Json<Value> {
    debug!(user_id = %identity.user_id, "T3 UI shell snapshot bootstrap");
    Json(empty_shell_snapshot())
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

fn origin_allowed(headers: &HeaderMap, allowed_origins: &[String]) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| allowed_origins.iter().any(|allowed| allowed == origin))
}

async fn serve_websocket(mut socket: WebSocket, state: UiRpcState, identity: AuthIdentity) {
    debug!(user_id = %identity.user_id, "T3 UI WebSocket connected");
    let mut session = ProtocolSession::default();

    while let Some(message) = socket.recv().await {
        let output = match message {
            Ok(Message::Text(text)) => session.handle_json(text.as_bytes(), &state),
            Ok(Message::Binary(bytes)) => session.handle_json(&bytes, &state),
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
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

        for frame in output.frames {
            let encoded = match serde_json::to_string(&frame) {
                Ok(encoded) => encoded,
                Err(error) => {
                    warn!(%error, "Failed to encode T3 UI RPC response");
                    return;
                }
            };
            if socket.send(Message::Text(encoded.into())).await.is_err() {
                return;
            }
        }

        if output.eof {
            break;
        }
    }

    debug!(user_id = %identity.user_id, "T3 UI WebSocket disconnected");
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
enum ClientFrame {
    Request {
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
        request_id: String,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    Ping,
    Eof,
}

#[derive(Debug, Default)]
struct StreamState {
    awaiting_ack: bool,
}

#[derive(Debug, Default)]
struct ProtocolSession {
    streams: HashMap<String, StreamState>,
}

#[derive(Debug, Default)]
struct ProtocolOutput {
    frames: Vec<Value>,
    eof: bool,
}

impl ProtocolSession {
    fn handle_json(&mut self, bytes: &[u8], state: &UiRpcState) -> ProtocolOutput {
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
                        .extend(self.handle_request(id, tag, payload, state));
                }
            }
        }
        output
    }

    fn handle_request(
        &mut self,
        id: String,
        tag: String,
        payload: Value,
        state: &UiRpcState,
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
                vec![success_exit(&id, empty_shell_snapshot())]
            }
            "orchestration.replayEvents" => vec![success_exit(&id, json!([]))],
            "orchestration.subscribeShell" => {
                let mut values = vec![json!({
                    "kind": "snapshot",
                    "snapshot": empty_shell_snapshot()
                })];
                if payload
                    .get("requestCompletionMarker")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    values.push(json!({ "kind": "synchronized" }));
                }
                self.start_stream(&id, Some(values))
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

    fn start_stream(&mut self, id: &str, values: Option<Vec<Value>>) -> Vec<Value> {
        if self.streams.len() >= MAX_ACTIVE_STREAMS {
            return vec![failure_exit(
                id,
                format!("At most {MAX_ACTIVE_STREAMS} RPC streams may be active"),
            )];
        }

        let awaiting_ack = values.is_some();
        self.streams
            .insert(id.to_string(), StreamState { awaiting_ack });
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

fn empty_shell_snapshot() -> Value {
    json!({
        "snapshotSequence": 0,
        "projects": [],
        "threads": [],
        "updatedAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

    fn state() -> UiRpcState {
        UiRpcState {
            rpc: Arc::new(UiRpcConfig {
                environment_id: "moatless-test".into(),
                label: "Moatless Test".into(),
                server_version: "0.0.28".into(),
                allowed_origins: vec!["http://localhost:5733".into()],
            }),
            session_cookie_name: Arc::from("moatless_session"),
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

    #[test]
    fn ping_returns_effect_rpc_pong() {
        let mut session = ProtocolSession::default();
        let output = session.handle_json(br#"{"_tag":"Ping"}"#, &state());
        assert_eq!(output.frames, vec![json!({ "_tag": "Pong" })]);
    }

    #[test]
    fn get_config_returns_minimal_decodable_wire_shape() {
        let mut session = ProtocolSession::default();
        let output = session.handle_json(&request("1", "server.getConfig", json!({})), &state());
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(
            output.frames[0]["exit"]["value"]["environment"]["environmentId"],
            "moatless-test"
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

    #[test]
    fn shell_subscription_chunks_snapshot_and_marker_then_waits_for_ack() {
        let mut session = ProtocolSession::default();
        let output = session.handle_json(
            &request(
                "2",
                "orchestration.subscribeShell",
                json!({ "requestCompletionMarker": true }),
            ),
            &state(),
        );
        assert_eq!(output.frames[0]["_tag"], "Chunk");
        assert_eq!(output.frames[0]["values"][0]["kind"], "snapshot");
        assert_eq!(output.frames[0]["values"][1]["kind"], "synchronized");
        assert!(session.streams["2"].awaiting_ack);

        let ack = session.handle_json(br#"{"_tag":"Ack","requestId":"2"}"#, &state());
        assert!(ack.frames.is_empty());
        assert!(!session.streams["2"].awaiting_ack);
    }

    #[test]
    fn interrupt_terminates_active_stream() {
        let mut session = ProtocolSession::default();
        session.handle_json(
            &request("3", "subscribeTerminalEvents", json!({})),
            &state(),
        );
        let output = session.handle_json(br#"{"_tag":"Interrupt","requestId":"3"}"#, &state());
        assert!(!session.streams.contains_key("3"));
        assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Interrupt");
    }

    #[test]
    fn a_json_array_is_one_effect_rpc_batch() {
        let mut session = ProtocolSession::default();
        let output = session.handle_json(
            br#"[{"_tag":"Ping"},{"_tag":"Request","id":"4","tag":"server.probe","payload":{},"headers":[]}]"#,
            &state(),
        );
        assert_eq!(output.frames.len(), 2);
        assert_eq!(output.frames[0]["_tag"], "Pong");
        assert_eq!(output.frames[1]["_tag"], "Exit");
    }

    #[test]
    fn origin_check_is_exact_and_requires_origin() {
        let allowed = vec!["http://localhost:5733".to_string()];
        let mut headers = HeaderMap::new();
        assert!(!origin_allowed(&headers, &allowed));
        headers.insert(header::ORIGIN, "http://localhost:5733".parse().unwrap());
        assert!(origin_allowed(&headers, &allowed));
        headers.insert(header::ORIGIN, "http://attacker.example".parse().unwrap());
        assert!(!origin_allowed(&headers, &allowed));
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
