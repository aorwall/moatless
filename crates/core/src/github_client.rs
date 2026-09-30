//! GitHub API client helpers for token validation and repository access.

use crate::git_provider::GitProviderError;

/// Validate a GitHub token by calling the user endpoint under `api_base`.
/// Returns the login (username) on success.
pub async fn validate_github_token(
    api_base: &str,
    token: &str,
) -> Result<String, GitProviderError> {
    let client = reqwest::Client::new();
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
    api_base: &str,
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<(), GitProviderError> {
    let client = reqwest::Client::new();
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
