//! Authentication flows: sandbox token exchange and browser-based OAuth login.

use reqwest::header::{HeaderMap, HeaderValue};
use std::path::Path;

use crate::client::{ClientError, MoatClient};
use crate::config::{AuthMode, Config};

// ============================================================================
// Sandbox mode: token exchange
// ============================================================================

/// Exchange the USER_AUTH_TOKEN for a session JWT, save to disk.
pub async fn login_sandbox(client: &mut MoatClient, config: &Config) -> Result<(), ClientError> {
    let user_auth_token = match &config.auth_mode {
        AuthMode::Sandbox { user_auth_token, .. } => user_auth_token.clone(),
        AuthMode::Standalone { .. } => {
            return Err(ClientError::Api {
                status: 500,
                body: "login_sandbox called in standalone mode".to_string(),
            });
        }
    };

    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&user_auth_token) {
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

/// Load session from disk, or auto-login if missing/expired (sandbox mode).
pub async fn ensure_session_sandbox(
    client: &mut MoatClient,
    config: &Config,
) -> Result<(), ClientError> {
    if let Some(token) = load_session(&config.session_file) {
        client.set_session_token(token);
        return Ok(());
    }
    login_sandbox(client, config).await
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

// ============================================================================
// Standalone mode: browser-based OAuth login
// ============================================================================

/// Browser-based login for standalone CLI usage.
///
/// 1. Bind a one-shot HTTP server on a random port
/// 2. Open browser to `{api_url}/api/v1/auth/cli/login?port={port}`
/// 3. Wait for callback with API key
/// 4. Save credentials to disk
pub async fn login_browser(config: &Config) -> Result<(), ClientError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Bind to a random available port
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| ClientError::Api {
            status: 500,
            body: format!("Failed to bind localhost server: {e}"),
        })?;
    let port = listener
        .local_addr()
        .map_err(|e| ClientError::Api {
            status: 500,
            body: format!("Failed to get local address: {e}"),
        })?
        .port();

    let login_url = format!(
        "{}/api/v1/auth/cli/login?port={port}",
        config.api_url.trim_end_matches('/')
    );

    println!("Opening browser to log in...");
    println!("If the browser doesn't open, visit: {login_url}");

    if let Err(e) = open::that(&login_url) {
        eprintln!("Failed to open browser: {e}");
        eprintln!("Please open the URL above manually.");
    }

    // Wait for callback with 120s timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let (mut stream, _addr) = listener.accept().await.map_err(|e| ClientError::Api {
            status: 500,
            body: format!("Failed to accept connection: {e}"),
        })?;

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.map_err(|e| ClientError::Api {
            status: 500,
            body: format!("Failed to read request: {e}"),
        })?;

        let request = String::from_utf8_lossy(&buf[..n]);
        let (api_key, login) = parse_callback_request(&request)?;

        // Send success response
        let html = "<!DOCTYPE html><html><body><h2>Login successful!</h2><p>You can close this tab and return to the terminal.</p></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;

        Ok::<(String, String), ClientError>((api_key, login))
    })
    .await;

    match result {
        Ok(Ok((api_key, login))) => {
            let host = config.host();
            crate::credentials::save_credential(&host, &api_key, &login).map_err(|e| {
                ClientError::Api {
                    status: 500,
                    body: format!("Failed to save credentials: {e}"),
                }
            })?;
            println!("Logged in as {login}");
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(ClientError::Api {
            status: 408,
            body: "Login timed out after 120 seconds".to_string(),
        }),
    }
}

/// Parse the callback request to extract `key` and `login` query params.
///
/// Expected: `GET /callback?key=mvk_...&login=albert HTTP/1.1`
fn parse_callback_request(request: &str) -> Result<(String, String), ClientError> {
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("");

    let query = path.split('?').nth(1).unwrap_or("");
    let mut key = None;
    let mut login = None;

    for param in query.split('&') {
        if let Some((k, v)) = param.split_once('=') {
            match k {
                "key" => key = Some(urlencoding::decode(v).unwrap_or_default().into_owned()),
                "login" => login = Some(urlencoding::decode(v).unwrap_or_default().into_owned()),
                _ => {}
            }
        }
    }

    let api_key = key.ok_or_else(|| ClientError::Api {
        status: 500,
        body: "No API key in callback".to_string(),
    })?;
    let user_login = login.unwrap_or_else(|| "unknown".to_string());

    if !api_key.starts_with("mvk_") {
        return Err(ClientError::Api {
            status: 500,
            body: "Invalid API key format in callback".to_string(),
        });
    }

    Ok((api_key, user_login))
}

// ============================================================================
// Ensure auth: dispatcher for both modes
// ============================================================================

/// Ensure we have valid auth. In sandbox mode, exchanges token for session.
/// In standalone mode, checks for API key and prompts login if missing.
pub async fn ensure_auth(client: &mut MoatClient, config: &Config) -> Result<(), ClientError> {
    match &config.auth_mode {
        AuthMode::Sandbox { .. } => ensure_session_sandbox(client, config).await,
        AuthMode::Standalone { api_key } => {
            if api_key.is_some() {
                Ok(())
            } else {
                Err(ClientError::NotLoggedIn)
            }
        }
    }
}
