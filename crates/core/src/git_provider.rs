//! The shared failure type for the HTTP git-provider credential clients
//! (`github_client`, `provider_settings::gitness_client`).

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
