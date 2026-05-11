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
    /// Required when the caller belongs to multiple teams.
    pub team_id: Option<String>,
    /// Access scope for this connection. Defaults to `user`, or `team` when
    /// only `teamId` is supplied.
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
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
