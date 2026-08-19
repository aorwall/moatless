//! Canonical authenticated-user type injected into request extensions by the auth middleware.

use uuid::Uuid;

use moatless_types::user::UserRole;

/// The authenticated user identity, set by the auth middleware on every
/// successfully-validated request.  Downstream extractors read this from
/// `request.extensions()` instead of headers.
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub user_id: Uuid,
    pub login: String,
    pub role: UserRole,
    pub provider: String,
    pub group_id: Option<String>,
}

/// Which credential the middleware accepted for the current request, injected
/// alongside [`AuthIdentity`].
///
/// Read by handlers whose defences depend on *how* the caller proved who they
/// are rather than on who they are — the WebSocket origin check being the one
/// that exists: a cookie is attached by the browser to any request anybody can
/// cause it to make, and the other two are attached only by a client that holds
/// them deliberately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    SessionCookie,
    ApiKey,
    /// A short-lived ticket in the query string, accepted only on the WebSocket
    /// upgrade. See [`crate::auth::jwt::generate_ws_ticket_jwt`].
    WebSocketTicket,
    /// A sandbox-issued `X-User-Token` JWT, carried by the agent's CLI from
    /// inside a sandbox pod. A deliberately-held bearer credential, so like an
    /// API key it is exempt from CSRF; it also marks the request as agent
    /// context for `source`-tagging downstream.
    UserToken,
}
