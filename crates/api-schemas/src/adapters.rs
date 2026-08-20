//! External adapter connection API schemas.

use serde::{Deserialize, Serialize};

use crate::enums::{AdapterKind, Scope};

/// Request body for `POST /api/v1/adapters/connections`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateAdapterConnectionRequest {
    /// Adapter implementation kind, such as `slack`, `github`, or `webhook`.
    pub adapter_kind: AdapterKind,
    /// Transport/auth style for this adapter, e.g. `bot_api`, `org_webhook`.
    pub connection_kind: Option<String>,
    /// External account/workspace/org identifier for this connection.
    pub external_account_id: String,
    /// HMAC secret used to verify inbound webhooks for this workspace.
    pub webhook_secret: Option<String>,
    /// Optional human-readable name for the connection.
    pub source_name: Option<String>,
    /// Optional transport-specific metadata.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub transport_config: Option<serde_json::Value>,
    /// Access scope for this connection. Defaults to `user`.
    pub scope: Option<Scope>,
}

/// One adapter connection, describing a transport/auth scope.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdapterConnectionResponse {
    pub id: String,
    pub adapter_kind: AdapterKind,
    pub connection_kind: String,
    pub external_account_id: String,
    pub created_by: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Request body for `POST /api/v1/adapters/send` — generic adapter pass-through send.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdapterSendRequest {
    /// Which adapter to dispatch through.
    pub adapter: AdapterKind,
    /// Optional connection ID. When omitted, the backend picks the single
    /// accessible connection of `adapter` kind, or returns an error if zero
    /// or more than one match.
    pub connection_id: Option<String>,
    /// Adapter-specific destination string. Each adapter defines its own
    /// format (e.g. Slack: `channel:C123` / `dm:D123` / `thread:C/ts`;
    /// Linear: `issue:LIN-123`).
    pub destination: String,
    /// Message text.
    pub text: String,
    /// File IDs (uploaded via `POST /api/v1/uploads`) to attach.
    #[serde(default)]
    pub file_ids: Vec<String>,
    /// Adapter-native rich rendering of the message body. The payload names
    /// the adapter it addresses; one whose kind does not match the resolved
    /// destination is rejected rather than dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<moatless_types::adapter::OutboundPresentation>,
    /// Optional task to bind this send to. When set, a `task_bindings` row
    /// is created so future `moat reply` calls thread under this destination.
    pub task_id: Option<String>,
}

/// Response from `POST /api/v1/adapters/send`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AdapterSendResponse {
    /// Adapter-specific external message ID (e.g. Slack `ts`, Linear comment ID).
    pub external_id: String,
    /// Binding `subject_kind` produced by the adapter (e.g. `slack_thread`,
    /// `linear_comment`). Surfaced so callers can correlate with bindings.
    pub subject_kind: String,
    /// ID of the binding created/updated by this send. Present only when
    /// `task_id` was provided on the request.
    pub binding_id: Option<String>,
}
