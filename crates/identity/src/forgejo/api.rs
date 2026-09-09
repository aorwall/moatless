//! Forgejo's REST API v1, as far as a credential check needs it.

use moatless_core::git_provider::GitProviderError;

use crate::forgejo::instances::base_url;

const OPERATION: &str = "Forgejo API";

/// Check that `token` still authenticates against `host`.
///
/// Probes `GET /api/v1/repos/search`, deliberately rather than
/// `GET /api/v1/user`: a Forgejo personal access token must name its scopes,
/// and the narrowest one worth storing here — `read:repository` — cannot reach
/// the `user` endpoint. Validating there would reject every correctly scoped
/// token and accept only over-scoped ones. The cost is that the check learns
/// no login, so a screen names the instance instead of an account.
///
/// The endpoint answers 200 unauthenticated, so only an outright rejection
/// counts: a token Forgejo ignores is a token Forgejo would ignore on a clone.
pub async fn validate_token(
    client: &reqwest::Client,
    host: &str,
    token: &str,
) -> Result<(), GitProviderError> {
    let base = base_url(host);
    let response = client
        .get(format!("{base}/api/v1/repos/search?limit=1"))
        .header("Authorization", format!("token {token}"))
        .send()
        .await
        .map_err(|source| GitProviderError::Unreachable {
            operation: OPERATION,
            source,
        })?;

    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(GitProviderError::Rejected {
        operation: OPERATION,
        status,
        body,
    })
}
