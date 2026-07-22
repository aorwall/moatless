//! Canonical authenticated-user type injected into request extensions by the auth middleware.

use uuid::Uuid;

use super::models::UserRole;

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
    pub team_ids: Vec<String>,
}
