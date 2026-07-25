//! Helpers for the canonical scoped-resource policy.
//!
//! The policy itself is pure and lives in
//! [`moatless_backend_core::scope::ScopeContext`]. Enforcement is
//! SQL-embedded: reads filter through `access_filter()`, mutations through
//! `control_filter()` inside the DAO statement. The helpers here cover the
//! two decisions that happen before a row exists: what scope a new resource
//! may be created with, and whether a repository may be used.

use crate::extractors::AuthUser;
use moatless_backend_core::scope::{Scope, ScopeContext, ScopedResource};

#[derive(Debug, thiserror::Error)]
pub enum AccessControlError {
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
}

#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub owner_user_id: Option<String>,
}

/// Resolve the scope for a new resource: defaults to `User` owned by the
/// caller; requesting `Global` requires admin.
pub fn resolve_scope_for_create(
    ctx: &ScopeContext,
    requested_scope: Option<Scope>,
) -> Result<ResolvedScope, AccessControlError> {
    let scope = requested_scope.unwrap_or(Scope::User);
    let owner_user_id = (scope == Scope::User).then(|| ctx.user_id.clone());

    if !ctx.can_create_scope(scope, owner_user_id.as_deref()) {
        return Err(AccessControlError::Forbidden(format!(
            "Cannot create {scope} scoped resources"
        )));
    }

    Ok(ResolvedScope {
        scope,
        owner_user_id,
    })
}

/// Convenience wrapper for HTTP handlers.
pub fn resolve_scope_for_create_auth(
    auth: &AuthUser,
    requested_scope: Option<Scope>,
) -> Result<ResolvedScope, AccessControlError> {
    resolve_scope_for_create(&auth.scope_context(), requested_scope)
}

pub fn ensure_can_use_repository(
    ctx: &ScopeContext,
    repository: &ScopedResource,
) -> Result<(), AccessControlError> {
    if ctx.can_use_repository(repository) {
        Ok(())
    } else {
        Err(AccessControlError::Forbidden(
            "No access to repository".to_string(),
        ))
    }
}
