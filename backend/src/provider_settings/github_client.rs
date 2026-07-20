//! GitHub API client helpers for PAT validation and OAuth token exchange.

/// Validate a GitHub token by calling the user endpoint.
/// Returns the login (username) on success.
pub async fn validate_github_token(token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/user", crate::adapters::github_pr::api::api_base()))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "moatless-vibe")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API returned {status}: {body}"));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse GitHub response: {e}"))?;

    data.get("login")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| "GitHub response missing login field".to_string())
}

/// Check whether a GitHub token has access to a specific repository.
/// Returns `Ok(())` if accessible, `Err` with a message otherwise.
pub async fn check_github_repo_access(token: &str, owner: &str, repo: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("https://api.github.com/repos/{owner}/{repo}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "moatless-vibe")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND
        || resp.status() == reqwest::StatusCode::FORBIDDEN
    {
        Err(format!(
            "You do not have access to the GitHub repository {owner}/{repo}. Please check your permissions."
        ))
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("GitHub API returned {status}: {body}"))
    }
}

/// Exchange a GitHub OAuth code for an access token.
/// Returns (access_token, token_type).
pub async fn exchange_github_oauth_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<(String, String), String> {
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
        .map_err(|e| format!("GitHub OAuth request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub OAuth returned {status}: {body}"));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse OAuth response: {e}"))?;

    if let Some(error) = data.get("error").and_then(|v| v.as_str()) {
        let desc = data
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("GitHub OAuth error: {error} — {desc}"));
    }

    let access_token = data
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("Missing access_token in OAuth response")?
        .to_string();

    let token_type = data
        .get("token_type")
        .and_then(|v| v.as_str())
        .unwrap_or("bearer")
        .to_string();

    Ok((access_token, token_type))
}
