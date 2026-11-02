//! GitHub API client helpers for token validation and repository access.

use moatless_types::http::HttpTimeouts;

use crate::git_provider::GitProviderError;

/// Sandbox preflight and token validation await these checks inline, so a
/// stalled GitHub has to resolve to an error.
#[expect(
    clippy::disallowed_methods,
    reason = "sets the connect and total timeouts `Config::http` holds"
)]
fn http_client(timeouts: HttpTimeouts) -> Result<reqwest::Client, GitProviderError> {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .build()
        .map_err(|source| GitProviderError::Unreachable {
            operation: "GitHub API",
            source,
        })
}

/// Validate a GitHub token by calling the user endpoint under `api_base`.
/// Returns the login (username) on success.
pub async fn validate_github_token(
    timeouts: HttpTimeouts,
    api_base: &str,
    token: &str,
) -> Result<String, GitProviderError> {
    let client = http_client(timeouts)?;
    let resp = client
        .get(format!("{api_base}/user"))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "moatless-vibe")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|source| GitProviderError::Unreachable {
            operation: "GitHub API",
            source,
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(GitProviderError::Rejected {
            operation: "GitHub API",
            status,
            body,
        });
    }

    let data: serde_json::Value =
        resp.json()
            .await
            .map_err(|source| GitProviderError::UndecodableResponse {
                operation: "GitHub",
                source,
            })?;

    data.get("login")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| {
            GitProviderError::MalformedResponse("GitHub response missing login field".to_string())
        })
}

/// Check whether a GitHub token has access to a specific repository. Returns
/// `Ok(())` if accessible, `Err` with a classified failure otherwise.
pub async fn check_github_repo_access(
    timeouts: HttpTimeouts,
    api_base: &str,
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<(), GitProviderError> {
    let client = http_client(timeouts)?;
    let resp = client
        .get(format!("{api_base}/repos/{owner}/{repo}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "moatless-vibe")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|source| GitProviderError::Unreachable {
            operation: "GitHub API",
            source,
        })?;

    if resp.status().is_success() {
        Ok(())
    } else if let Some(limited) = rate_limit(resp.status(), resp.headers(), chrono::Utc::now()) {
        Err(limited)
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND
        || resp.status() == reqwest::StatusCode::FORBIDDEN
    {
        Err(GitProviderError::RepositoryForbidden {
            // The only host `parse_github` accepts, whatever API base this
            // deployment calls.
            host: "github.com".to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(GitProviderError::Rejected {
            operation: "GitHub API",
            status,
            body,
        })
    }
}

/// [`GitProviderError::RateLimited`] when GitHub refused the request for its
/// rate limit rather than for access.
///
/// GitHub answers a spent limit with 403 or 429, so a 403 alone does not tell
/// the two apart: the `x-ratelimit-remaining: 0` or `retry-after` header does.
fn rate_limit(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<GitProviderError> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<i64>().ok())
    };
    let retry_after = header("retry-after");
    let spent = header("x-ratelimit-remaining") == Some(0);
    let limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || (status == reqwest::StatusCode::FORBIDDEN && (spent || retry_after.is_some()));
    if !limited {
        return None;
    }
    let retry_at = match retry_after {
        Some(seconds) => Some(now + chrono::Duration::seconds(seconds)),
        None if spent => {
            header("x-ratelimit-reset").and_then(|epoch| chrono::DateTime::from_timestamp(epoch, 0))
        }
        None => None,
    };
    Some(GitProviderError::RateLimited {
        operation: "GitHub API",
        retry_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{DateTime, Utc};
    use reqwest::StatusCode;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    /// The `retry_at` of an answer that must be a rate limit.
    fn limited_until(status: StatusCode, headers: &HeaderMap) -> Option<DateTime<Utc>> {
        match rate_limit(status, headers, now()) {
            Some(GitProviderError::RateLimited { retry_at, .. }) => retry_at,
            other => panic!("not a rate limit: {other:?}"),
        }
    }

    #[test]
    fn a_403_with_a_spent_primary_limit_is_a_rate_limit_until_its_reset() {
        let headers = headers(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1800001000"),
        ]);

        assert_eq!(
            limited_until(StatusCode::FORBIDDEN, &headers),
            DateTime::from_timestamp(1_800_001_000, 0),
        );
    }

    #[test]
    fn a_secondary_limit_retries_after_its_retry_after_seconds() {
        let headers = headers(&[("retry-after", "60"), ("x-ratelimit-remaining", "12")]);

        assert_eq!(
            limited_until(StatusCode::FORBIDDEN, &headers),
            Some(now() + chrono::Duration::seconds(60)),
        );
    }

    #[test]
    fn a_429_without_headers_is_a_rate_limit_with_no_known_end() {
        assert_eq!(
            limited_until(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new()),
            None,
        );
    }

    #[test]
    fn a_403_with_budget_left_is_an_access_refusal() {
        let headers = headers(&[("x-ratelimit-remaining", "4999")]);

        assert!(rate_limit(StatusCode::FORBIDDEN, &headers, now()).is_none());
        assert!(rate_limit(StatusCode::NOT_FOUND, &headers, now()).is_none());
    }
}
