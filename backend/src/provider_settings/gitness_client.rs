//! Gitness API client helpers for PAT validation.

/// Validate a Gitness token by calling the user endpoint.
/// Returns the login (username) on success.
pub async fn validate_gitness_token(base_url: &str, token: &str) -> Result<String, String> {
    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{base}/api/v1/user"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Gitness API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Gitness API returned {status}: {body}"));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gitness response: {e}"))?;

    // Gitness user response has "login" or "uid" field
    data.get("login")
        .or_else(|| data.get("uid"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| "Gitness response missing login/uid field".to_string())
}
