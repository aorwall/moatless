//! GitHub API client helpers for PAT validation and OAuth token exchange.

use super::error::GitProviderError;

/// Validate a GitHub token by calling the user endpoint.
/// Returns the login (username) on success.
pub async fn validate_github_token(token: &str) -> Result<String, GitProviderError> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/user",
            crate::adapters::github_pr::api::api_base()
        ))
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

/// Check whether a GitHub token has access to a specific repository.
/// Returns `Ok(())` if accessible, `Err` with a classified failure otherwise.
pub async fn check_github_repo_access(
    token: &str,
    owner: &str,
    repo: &str,
) -> Result<(), GitProviderError> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/repos/{owner}/{repo}",
            crate::adapters::github_pr::api::api_base()
        ))
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

/// Exchange a GitHub OAuth code for an access token.
/// Returns (access_token, token_type).
pub async fn exchange_github_oauth_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<(String, String), GitProviderError> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .header("User-Agent", "moatless-vibe")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
        ])
        .send()
        .await
        .map_err(|source| GitProviderError::Unreachable {
            operation: "GitHub OAuth",
            source,
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(GitProviderError::Rejected {
            operation: "GitHub OAuth",
            status,
            body,
        });
    }

    let data: serde_json::Value =
        resp.json()
            .await
            .map_err(|source| GitProviderError::UndecodableResponse {
                operation: "OAuth",
                source,
            })?;

    if let Some(error) = data.get("error").and_then(|v| v.as_str()) {
        let desc = data
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(GitProviderError::OAuthRejected {
            code: error.to_string(),
            description: desc.to_string(),
        });
    }

    let access_token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            GitProviderError::MalformedResponse(
                "Missing access_token in OAuth response".to_string(),
            )
        })?
        .to_string();

    let token_type = data
        .get("token_type")
        .and_then(|v| v.as_str())
        .unwrap_or("bearer")
        .to_string();

    Ok((access_token, token_type))
}
