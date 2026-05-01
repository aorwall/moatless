//! Credential service error type.

use crate::auth::error::AuthError;

/// Errors returned by [`crate::credentials::CredentialService`].
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("auth error: {0}")]
    Auth(#[from] AuthError),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("internal: {0}")]
    Internal(String),
}
