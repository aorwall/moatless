//! User provider ID management and DM send request/response types.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Set or replace the provider-specific user ID for a moatless user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SetProviderIdRequest {
    /// Provider-specific user ID (e.g. Slack user ID "U0000000002").
    pub provider_user_id: String,
}

/// A single provider ID mapping entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UserProviderIdItem {
    pub provider: String,
    pub provider_user_id: String,
    pub created_at: String,
}

/// Send a DM to a user via an external provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SendDmRequest {
    /// Provider to send through (e.g. "slack").
    pub provider: String,
    /// Message text.
    pub text: String,
    /// Optional task to associate the DM binding with.
    pub task_id: Option<String>,
}

/// Result of sending a DM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SendDmResponse {
    /// The created or updated task_binding id.
    pub binding_id: String,
    /// DM channel ID opened (Slack: starts with "D").
    pub channel_id: String,
    /// Message timestamp (Slack ts).
    pub message_ts: String,
}
