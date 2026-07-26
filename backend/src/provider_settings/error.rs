//! Provider settings error types.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use crate::error::error_response;

/// Failure from a git-provider credential check (GitHub or Gitness).
///
/// Classification matters to the caller: a token that a provider *rejected* is
/// the user's problem, while a provider we could not *reach* is ours. Both keep
/// the causal error reachable through [`std::error::Error::source`].
#[derive(Debug, thiserror::Error)]
pub enum GitProviderError {
    /// We never got an answer — DNS, TLS, connect, or timeout.
    #[error("{operation} request failed: {source}")]
    Unreachable {
        /// The provider call that failed, e.g. `GitHub API`.
        operation: &'static str,
        #[source]
        source: reqwest::Error,
    },

    /// The provider answered with a non-success status. Synthesized from the
    /// provider's own status line and body, so there is no source to carry.
    #[error("{operation} returned {status}: {body}")]
    Rejected {
        operation: &'static str,
        status: reqwest::StatusCode,
        body: String,
    },

    /// The token authenticates but has no access to the requested repository.
    #[error(
        "You do not have access to the GitHub repository {owner}/{repo}. \
         Please check your permissions."
    )]
    RepositoryForbidden { owner: String, repo: String },

    /// The provider answered, but the body is not decodable JSON.
    #[error("Failed to parse {operation} response: {source}")]
    UndecodableResponse {
        operation: &'static str,
        #[source]
        source: reqwest::Error,
    },

    /// The body decoded but is missing a field we require. Synthesized — there
    /// is no underlying error, only an absent field.
    #[error("{0}")]
    MalformedResponse(String),

    /// The OAuth endpoint answered 2xx with a structured error payload.
    #[error("GitHub OAuth error: {code} — {description}")]
    OAuthRejected { code: String, description: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderSettingsError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// A PAT/OAuth check against GitHub or Gitness failed.
    #[error("Provider error: {0}")]
    GitProvider(#[source] GitProviderError),

    /// A GitHub App API call (installations, app listing) failed.
    #[error("Provider error: {0}")]
    GitHubApp(#[source] crate::auth::error::AuthError),

    /// A Codex device-code login step failed.
    #[error("Provider error: {0}")]
    CodexOAuth(#[source] crate::credentials::codex::oauth::CodexOAuthError),

    #[error("Secret storage error: {0}")]
    Secret(#[from] crate::secret::error::SecretError),
}

impl IntoResponse for ProviderSettingsError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            Self::Db(e) => {
                error!(error = %e, "Provider settings database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
            Self::GitProvider(_) | Self::GitHubApp(_) | Self::CodexOAuth(_) => {
                (StatusCode::BAD_GATEWAY, self.to_string())
            }
            Self::Secret(e) => {
                error!(error = %e, "Provider settings secret storage error");
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
    use std::error::Error as _;

    use super::*;
    use crate::adapters::test_support::{
        response_snapshot, stub_provider, transport_error, UNREACHABLE_BASE,
    };
    use crate::provider_settings::gitness_client::validate_gitness_token;

    #[tokio::test]
    async fn unreachable_provider_keeps_transport_source() {
        // `validate_gitness_token` takes its base URL as an argument, so this
        // drives the real request path used by every git-provider PAT check.
        let err = validate_gitness_token(UNREACHABLE_BASE, "pat")
            .await
            .expect_err("dead loopback port must fail");

        let GitProviderError::Unreachable {
            operation,
            ref source,
        } = err
        else {
            panic!("expected Unreachable, got {err:?}");
        };
        assert_eq!(operation, "Gitness API");
        assert!(source.is_connect() || source.is_request(), "{source}");
        assert!(err.source().is_some(), "cause must stay reachable");
    }

    /// A provider that answered and said no is synthesized from its own status
    /// and body — no source, and a different variant from "unreachable".
    #[tokio::test]
    async fn rejected_provider_stays_a_synthesized_message() {
        let base = stub_provider(StatusCode::UNAUTHORIZED, "bad token").await;

        let err = validate_gitness_token(&base, "pat")
            .await
            .expect_err("401 must be an error");

        assert!(
            matches!(err, GitProviderError::Rejected { status, .. } if status == StatusCode::UNAUTHORIZED),
            "got {err:?}"
        );
        assert!(err.source().is_none());
    }

    #[tokio::test]
    async fn undecodable_provider_body_keeps_decode_source() {
        let base = stub_provider(StatusCode::OK, "<html>not json</html>").await;

        let err = validate_gitness_token(&base, "pat")
            .await
            .expect_err("html body must not decode");

        assert!(
            matches!(err, GitProviderError::UndecodableResponse { .. }),
            "got {err:?}"
        );
        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<reqwest::Error>())
            .is_some());
    }

    #[tokio::test]
    async fn git_provider_failure_is_a_502_carrying_the_provider_words() {
        let err = ProviderSettingsError::GitProvider(GitProviderError::Rejected {
            operation: "GitHub API",
            status: StatusCode::UNAUTHORIZED,
            body: "Bad credentials".to_string(),
        });

        assert!(err.source().is_some(), "cause must stay reachable");

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            body,
            r#"{"error":"Provider error: GitHub API returned 401 Unauthorized: Bad credentials"}"#
        );
    }

    #[tokio::test]
    async fn github_app_failure_keeps_auth_source_on_the_502_path() {
        let err = ProviderSettingsError::GitHubApp(crate::auth::error::AuthError::Config(
            "app key missing".to_string(),
        ));

        assert!(err
            .source()
            .and_then(|s| s.downcast_ref::<crate::auth::error::AuthError>())
            .is_some());

        let (status, body) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            body,
            r#"{"error":"Provider error: Configuration error: app key missing"}"#
        );
    }

    #[tokio::test]
    async fn codex_oauth_failure_keeps_source_on_the_502_path() {
        let err = ProviderSettingsError::CodexOAuth(
            crate::credentials::codex::oauth::CodexOAuthError::Network(transport_error().await),
        );

        assert!(err.source().is_some(), "cause must stay reachable");

        let (status, _) = response_snapshot(err.into_response()).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
    }

    /// The database and secret paths keep their redaction: nothing about SQL or
    /// credential storage reaches the client.
    #[tokio::test]
    async fn internal_failures_stay_redacted() {
        let (status, body) =
            response_snapshot(ProviderSettingsError::Db(sqlx::Error::PoolClosed).into_response())
                .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }
}
