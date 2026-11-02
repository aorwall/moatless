//! GitHub API client helpers for token validation and repository access.

use chrono::{DateTime, Utc};
use moatless_types::http::HttpTimeouts;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};

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

    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let headers = resp.headers().clone();
    let body = resp.text().await.unwrap_or_default();
    if let Some(throttle) = github_throttle(status, &headers, &body, Utc::now()) {
        Err(GitProviderError::RateLimited {
            operation: "GitHub API",
            retry_at: throttle.retry_at,
        })
    } else if status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN {
        Err(GitProviderError::RepositoryForbidden {
            // The only host `parse_github` accepts, whatever API base this
            // deployment calls.
            host: "github.com".to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    } else {
        Err(GitProviderError::Rejected {
            operation: "GitHub API",
            status,
            body,
        })
    }
}

const RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";
const RATE_LIMIT_RESET: &str = "x-ratelimit-reset";

/// GitHub refused a request for its rate limit, not for access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GithubThrottle {
    /// When GitHub accepts requests again, if it said.
    pub retry_at: Option<DateTime<Utc>>,
}

/// `Some` when a non-success answer is GitHub's primary or secondary rate
/// limit: a 429, or a 403 with `retry-after`, `x-ratelimit-remaining: 0` or
/// "rate limit" in the body.
///
/// Every GitHub client classifies through this. A 403 alone does not tell a
/// spent quota from a missing permission.
pub fn github_throttle(
    status: StatusCode,
    headers: &HeaderMap,
    body: &str,
    now: DateTime<Utc>,
) -> Option<GithubThrottle> {
    let limited = status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (headers.contains_key(RETRY_AFTER)
                || header(headers, RATE_LIMIT_REMAINING) == Some("0")
                || body.to_ascii_lowercase().contains("rate limit")));
    limited.then(|| GithubThrottle {
        retry_at: github_retry_at(headers, now),
    })
}

/// When GitHub's quota returns: `retry-after` in seconds, else
/// `x-ratelimit-reset` in epoch seconds once `x-ratelimit-remaining` is zero.
///
/// `None` on an answer with quota left, whose reset header is not a refusal's.
pub fn github_retry_at(headers: &HeaderMap, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if let Some(seconds) =
        header(headers, RETRY_AFTER.as_str()).and_then(|value| value.parse::<i64>().ok())
    {
        return Some(now + chrono::Duration::seconds(seconds.max(0)));
    }
    if header(headers, RATE_LIMIT_REMAINING) != Some("0") {
        return None;
    }
    header(headers, RATE_LIMIT_RESET)
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|epoch| DateTime::from_timestamp(epoch, 0))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn throttle(status: StatusCode, headers: &HeaderMap, body: &str) -> Option<GithubThrottle> {
        github_throttle(status, headers, body, now())
    }

    #[test]
    fn a_spent_primary_limit_is_throttled_until_its_reset() {
        let map = headers(&[
            (RATE_LIMIT_REMAINING, "0"),
            (RATE_LIMIT_RESET, "1800001000"),
        ]);

        assert_eq!(
            throttle(StatusCode::FORBIDDEN, &map, ""),
            Some(GithubThrottle {
                retry_at: DateTime::from_timestamp(1_800_001_000, 0),
            }),
        );
    }

    #[test]
    fn a_secondary_limit_waits_its_retry_after() {
        let map = headers(&[("retry-after", "60"), (RATE_LIMIT_REMAINING, "12")]);

        assert_eq!(
            throttle(StatusCode::FORBIDDEN, &map, ""),
            Some(GithubThrottle {
                retry_at: Some(now() + chrono::Duration::seconds(60)),
            }),
        );
    }

    /// GitHub can answer a secondary limit with no timing header at all.
    #[test]
    fn a_403_whose_body_names_a_rate_limit_is_throttled_with_no_known_end() {
        let body = r#"{"message":"You have exceeded a secondary rate limit."}"#;

        assert_eq!(
            throttle(StatusCode::FORBIDDEN, &HeaderMap::new(), body),
            Some(GithubThrottle { retry_at: None }),
        );
        assert_eq!(
            throttle(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new(), ""),
            Some(GithubThrottle { retry_at: None }),
        );
    }

    #[test]
    fn a_negative_retry_after_retries_now() {
        let map = headers(&[("retry-after", "-5")]);

        assert_eq!(github_retry_at(&map, now()), Some(now()));
    }

    #[test]
    fn a_permission_refusal_with_quota_left_is_not_throttled() {
        let map = headers(&[
            (RATE_LIMIT_REMAINING, "4100"),
            (RATE_LIMIT_RESET, "1800000600"),
        ]);
        let body = r#"{"message":"Resource not accessible by integration"}"#;

        assert_eq!(throttle(StatusCode::FORBIDDEN, &map, body), None);
        assert_eq!(throttle(StatusCode::NOT_FOUND, &map, ""), None);
        assert_eq!(github_retry_at(&map, now()), None);
    }
}
