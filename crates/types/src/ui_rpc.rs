//! The identity the auth layer publishes for the UI RPC surface.
//!
//! These types travel in request extensions: the auth middleware inserts them,
//! the RPC handlers read them — both sides sit in different crates, so the
//! shape they agree on is owned here.

use uuid::Uuid;

/// A transport value, serialized to `"admin"`/`"user"` in a ticket claim.
///
/// Not the access-control input, which is
/// [`crate::access_control::ScopeContext::is_admin`], derived from this role
/// when a policy decision is needed.
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
    pub group_id: Option<String>,
}

/// How the caller proved who they are. The RPC layer reads it to decide whether
/// an origin check applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRpcAuthMethod {
    SessionCookie,
    ApiKey,
    WebSocketTicket,
}
