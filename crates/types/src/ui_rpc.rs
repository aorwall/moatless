//! The identity the auth layer publishes for the UI RPC surface, and the
//! settings the backend `Config` reads for it.
//!
//! The identity types travel in request extensions: the auth middleware inserts
//! them, the RPC handlers read them — both sides sit in different crates, so the
//! shape they agree on is owned here.

use std::fmt;

use uuid::Uuid;

use crate::auth::AllowedOrigin;

/// A transport value, serialized to `"admin"`/`"user"` in a ticket claim.
///
/// Not the access-control input, which is
/// [`crate::access_control::ScopeContext::is_admin`], derived from this role
/// when a policy decision is needed.
///
/// Deliberately not `Copy`: nothing here needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserRole {
    Admin,
    User,
}

crate::string_enum_mapping!(
    UserRole {
        Admin => "admin",
        User => "user",
    }
);

#[derive(Debug, Clone)]
pub struct UiRpcAuthIdentity {
    pub user_id: Uuid,
    pub login: String,
    pub role: UserRole,
    pub provider: String,
}

/// How the caller proved who they are. The RPC layer reads it to decide whether
/// an origin check applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRpcAuthMethod {
    SessionCookie,
    ApiKey,
    WebSocketTicket,
}

/// The T3 UI-RPC router's settings; `enabled` false leaves it unmounted.
#[derive(Clone, Default)]
pub struct T3UiRpcConfig {
    /// `T3_UI_RPC_ENABLED`.
    pub enabled: bool,
    /// `T3_UI_RPC_ALLOWED_ORIGINS`; none when unset.
    pub allowed_origins: Vec<AllowedOrigin>,
    /// `T3_CLIENT_OTLP_TRACES_ENDPOINT`, an OTLP/HTTP traces URL such as
    /// `http://otel-collector:4318/v1/traces`.
    pub client_traces_endpoint: Option<String>,
    /// `T3_UI_RPC_CLIPROXY_URL`, the `CLIProxyAPI` hub, as set.
    pub cliproxy_url: Option<String>,
    /// `T3_UI_RPC_CLIPROXY_MANAGEMENT_KEY`, as set.
    pub cliproxy_management_key: Option<String>,
    /// `T3_UI_RPC_CLIPROXY_LABEL`, as set.
    pub cliproxy_label: Option<String>,
    /// `T3_UI_RPC_SCRATCH_WORKSPACE`: the id of the Workspace a thread with no
    /// project runs in. Unset, a viewer's only readable blank Workspace is it.
    pub scratch_workspace: Option<String>,
    /// `T3_UI_RPC_LINEAR_DEFAULT_TEAM`: the Linear team key (`SP`) a Workspace
    /// declaring no `linearTeam` lists issues for. Unset, such a Workspace has
    /// no Linear team.
    pub linear_default_team: Option<String>,
}

impl fmt::Debug for T3UiRpcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("T3UiRpcConfig")
            .field("enabled", &self.enabled)
            .field("allowed_origins", &self.allowed_origins)
            .field("client_traces_endpoint", &self.client_traces_endpoint)
            .field("cliproxy_url", &self.cliproxy_url)
            .field(
                "cliproxy_management_key",
                &self.cliproxy_management_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("cliproxy_label", &self.cliproxy_label)
            .field("scratch_workspace", &self.scratch_workspace)
            .field("linear_default_team", &self.linear_default_team)
            .finish()
    }
}
