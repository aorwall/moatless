//! Token exchange and session file management.

use reqwest::header::{HeaderMap, HeaderValue};
use std::path::Path;

use crate::client::{ClientError, MoatClient};
use crate::config::Config;

/// Exchange the USER_AUTH_TOKEN for a session JWT, save to disk.
pub async fn login(client: &mut MoatClient, config: &Config) -> Result<(), ClientError> {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(client.user_auth_token()) {
        headers.insert("X-User-Token", v);
    }

    let resp = client
        .post_with_headers("/api/v1/auth/token/session", headers)
        .await?;

    let token = resp
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ClientError::Api {
            status: 500,
            body: "No token in response".to_string(),
        })?;

    // Save session to file
    std::fs::write(&config.session_file, token).map_err(|e| ClientError::Api {
        status: 500,
        body: format!("Failed to write session file: {e}"),
    })?;

    client.set_session_token(token.to_string());
    Ok(())
}

/// Load session from disk, or auto-login if missing/expired.
pub async fn ensure_session(client: &mut MoatClient, config: &Config) -> Result<(), ClientError> {
    // Try loading from file
    if let Some(token) = load_session(&config.session_file) {
        client.set_session_token(token);
        return Ok(());
    }

    // Auto-login
    login(client, config).await
}

fn load_session(path: &str) -> Option<String> {
    let path = Path::new(path);
    if !path.exists() {
        return None;
    }
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(token)
}
