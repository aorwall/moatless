//! The HTTP client the Forgejo endpoints and the OAuth exchange share.

/// An HTTP client for a Forgejo host's own endpoints.
///
/// Carries [`moatless_core::USER_AGENT`], required by a host behind a filtering
/// proxy for token validation and the OAuth exchange alike.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to build the Forgejo HTTP client");
            reqwest::Client::new()
        })
}
