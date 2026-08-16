//! The identity the auth layer publishes for the UI RPC surface.
//!
//! These types travel in request extensions: the auth middleware inserts them,
//! the RPC handlers read them. Both sides sit in different crates, so the shape
//! they agree on is owned here.
//!
//! `UserRole` is a transport value — it serializes to `"admin"` or `"user"` in
//! a ticket claim. It is not the access-control input; that is
//! [`crate::scope::ScopeContext::is_admin`], which the RPC layer derives from
//! this role at the point it needs a policy decision.

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserRole {
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiRpcAuthIdentity {
    pub user_id: Uuid,
    pub login: String,
    pub role: UserRole,
    pub provider: String,
    pub group_id: Option<String>,
}

/// How the caller proved who they are. The RPC layer reads it to decide
/// whether an origin check applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRpcAuthMethod {
    SessionCookie,
    ApiKey,
    WebSocketTicket,
}
