//! Forgejo API client helpers for repository access.
//!
//! Every call names the instance the repository's remote carried: Forgejo is
//! self-hosted, so there is no one API root to address.

use moatless_types::http::HttpTimeouts;

use crate::git_provider::GitProviderError;

/// The `operation` a Forgejo API call reports in a [`GitProviderError`].
pub const OPERATION: &str = "Forgejo API";

/// Bound on the error body kept from an unsuccessful answer. The repository's
/// own remote names the host, so its response is not ours to read unbounded.
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

/// Check whether `token` can read `owner/repo` on the Forgejo instance at
/// `host`, through `GET /api/v1/repos/{owner}/{repo}`.
///
/// A repository the token cannot see answers 404 rather than 403, so both are
/// [`GitProviderError::RepositoryForbidden`]. An instance behind a filtering
/// proxy requires the [`crate::USER_AGENT`] this sends.
#[expect(
    clippy::disallowed_methods,
    reason = "sets the connect and total timeouts `Config::http` holds"
)]
pub async fn check_forgejo_repo_access(
    timeouts: HttpTimeouts,
    host: &str,
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<(), GitProviderError> {
    let client = reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .user_agent(crate::USER_AGENT)
        .build()
        .map_err(|source| GitProviderError::Unreachable {
            operation: OPERATION,
            source,
        })?;
    let resp = client
        .get(format!("https://{host}/api/v1/repos/{owner}/{repo}"))
        .header("Authorization", format!("token {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|source| GitProviderError::Unreachable {
            operation: OPERATION,
            source,
        })?;

    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::FORBIDDEN {
        return Err(GitProviderError::RepositoryForbidden {
            host: host.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
        });
    }
    // 401 and every 5xx keep the host's own answer: a rejected token and an
    // instance in trouble are different remedies from "no access to this repo".
    let body = read_body_prefix(resp, MAX_ERROR_BODY_BYTES).await;
    Err(GitProviderError::Rejected {
        operation: OPERATION,
        status,
        body,
    })
}

/// Read at most `limit` bytes of `resp`'s body, never buffering past it, and
/// leave the rest undrained.
///
/// A body that stops early, fails mid-stream or is not UTF-8 degrades to what
/// arrived: the status it accompanies is the part a reader acts on, and losing
/// it to an unreadable explanation would be the worse trade.
async fn read_body_prefix(mut resp: reqwest::Response, limit: usize) -> String {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < limit {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let take = chunk.len().min(limit - buf.len());
                buf.extend_from_slice(&chunk[..take]);
            }
            Ok(None) | Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}
