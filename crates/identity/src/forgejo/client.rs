//! The HTTP client the Forgejo endpoints and the OAuth exchange share.

use moatless_types::http::HttpTimeouts;

/// An HTTP client for a Forgejo host's own endpoints.
///
/// Carries [`moatless_core::USER_AGENT`], required by a host behind a filtering
/// proxy for token validation and the OAuth exchange alike.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
pub(crate) fn http_client(timeouts: HttpTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .expect("the TLS backend initialises")
}
