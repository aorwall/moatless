//! HTTP error mapping for access administration.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use moatless_core::error::error_response;
use moatless_identity::adapter_apps::administration::AppAdministrationError;

#[derive(Debug, thiserror::Error)]
pub enum AccessSettingsError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Forbidden")]
    AuthForbidden,
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Database error: {0}")]
    Db(#[source] sqlx::Error),
    #[error("Database error: {0}")]
    Store(#[source] moatless_types::store::StoreError),
    #[error("Provider error: {0}")]
    GitProvider(#[source] moatless_core::git_provider::GitProviderError),
    #[error("Provider error: {0}")]
    GitHubApp(#[source] moatless_identity::auth::error::AuthError),
    #[error("Provider error: {0}")]
    CodexOAuth(#[source] moatless_identity::credentials::codex::oauth::CodexOAuthError),
    #[error("Secret storage error: {0}")]
    Secret(#[source] moatless_identity::secret::error::SecretError),
}

impl From<moatless_identity::git_host_access::GitHostAccessError> for AccessSettingsError {
    fn from(error: moatless_identity::git_host_access::GitHostAccessError) -> Self {
        use moatless_identity::git_host_access::GitHostAccessError;
        match error {
            GitHostAccessError::NotFound(message) => Self::NotFound(message),
            GitHostAccessError::BadRequest(message) => Self::BadRequest(message),
            GitHostAccessError::Db(source) => Self::Db(source),
            GitHostAccessError::Provider(source) => Self::GitProvider(source),
            GitHostAccessError::Secret(source) => Self::Secret(source),
        }
    }
}

impl From<crate::agent_harness_access::AgentHarnessAccessError> for AccessSettingsError {
    fn from(error: crate::agent_harness_access::AgentHarnessAccessError) -> Self {
        use crate::agent_harness_access::AgentHarnessAccessError;
        match error {
            AgentHarnessAccessError::Forbidden(message) => Self::Forbidden(message),
            AgentHarnessAccessError::AuthForbidden => Self::AuthForbidden,
            AgentHarnessAccessError::BadRequest(message) => Self::BadRequest(message),
            AgentHarnessAccessError::Db(source) => Self::Db(source),
            AgentHarnessAccessError::Store(source) => Self::Store(source),
            AgentHarnessAccessError::OAuth(source) => Self::CodexOAuth(source),
        }
    }
}

impl From<AppAdministrationError> for AccessSettingsError {
    fn from(error: AppAdministrationError) -> Self {
        match error {
            AppAdministrationError::NotFound(message) => Self::NotFound(message),
            AppAdministrationError::BadRequest(message) => Self::BadRequest(message),
            AppAdministrationError::Conflict(message) => Self::Conflict(message),
            AppAdministrationError::Forbidden(message) => Self::Forbidden(message),
            AppAdministrationError::Db(source) => Self::Db(source),
            AppAdministrationError::Store(source) => Self::Store(source),
            AppAdministrationError::GitHubApp(source) => Self::GitHubApp(source),
        }
    }
}

impl IntoResponse for AccessSettingsError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Forbidden(_) | Self::AuthForbidden => (StatusCode::FORBIDDEN, self.to_string()),
            Self::Conflict(_) => (StatusCode::CONFLICT, self.to_string()),
            Self::Db(source) => {
                error!(%source, "Access settings database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::Store(source) => {
                error!(%source, "Access settings database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::GitProvider(_) | Self::GitHubApp(_) | Self::CodexOAuth(_) => {
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            Self::Secret(source) => {
                error!(%source, "Access settings secret storage error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to store credential securely".to_string(),
                )
            }
        };
        error_response(status, message)
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn snapshot(error: AccessSettingsError) -> (StatusCode, String) {
        let response = error.into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn provider_failure_keeps_the_existing_502_body() {
        let error = AccessSettingsError::GitProvider(
            moatless_core::git_provider::GitProviderError::Rejected {
                operation: "GitHub API",
                status: StatusCode::UNAUTHORIZED,
                body: "Bad credentials".to_string(),
            },
        );

        assert_eq!(
            snapshot(error).await,
            (
                StatusCode::BAD_GATEWAY,
                r#"{"error":"Provider error: GitHub API returned 401 Unauthorized: Bad credentials"}"#
                    .to_string(),
            )
        );
    }

    #[tokio::test]
    async fn internal_failures_stay_redacted() {
        assert_eq!(
            snapshot(AccessSettingsError::Db(sqlx::Error::PoolClosed)).await,
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error":"Internal server error"}"#.to_string(),
            )
        );
    }
}
