//! Authentication flows: sandbox token exchange and browser-based OAuth login.

use std::path::Path;

use crate::client::{ClientError, MoatClient};
use crate::config::{AuthMode, Config, LoginEnv};

// ============================================================================
// Sandbox mode: token exchange
// ============================================================================

/// Exchange the `USER_AUTH_TOKEN` for a session JWT, save to disk.
pub async fn login_sandbox(client: &mut MoatClient, config: &Config) -> Result<(), ClientError> {
    if !matches!(config.auth_mode, AuthMode::Sandbox { .. }) {
        return Err(ClientError::Api {
            status: 500,
            body: "login_sandbox called in standalone mode".to_string(),
        });
    }

    let session = client.api().exchange_token().await?;

    // Save session to file
    std::fs::write(&config.session_file, &session.token).map_err(|e| ClientError::Api {
        status: 500,
        body: format!("Failed to write session file: {e}"),
    })?;

    client.set_session_token(session.token.clone());
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

/// Drop the cached sandbox session so the next invocation re-exchanges the
/// (refreshed) user token.
///
/// Called on a 401 — replaying a stale session would wedge the CLI in a loop.
/// No-op outside sandbox mode.
pub fn invalidate_session(config: &Config) {
    if matches!(config.auth_mode, AuthMode::Sandbox { .. }) {
        let _ = std::fs::remove_file(&config.session_file);
    }
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
// Standalone mode: login dispatcher (password vs OAuth)
// ============================================================================

/// Top-level login entry point.
///
/// Asks `{url}/api/v1/auth/mode` which flow the server runs, then prompts for a
/// password or opens the browser for OAuth; falls back to `config`'s
/// (env-resolved) URL when `url_override` is `None`.
pub async fn login(config: &Config, url_override: Option<String>) -> Result<(), ClientError> {
    let url = url_override
        .unwrap_or_else(|| config.api_url.clone())
        .trim_end_matches('/')
        .to_string();

    match probe_auth_mode(&url).await {
        ProbedAuthMode::Password => login_password(&url, &config.login).await,
        ProbedAuthMode::OAuth => login_browser(&url).await,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ProbedAuthMode {
    Password,
    OAuth,
}

/// Ask the server which login flow it runs.
///
/// Fails closed to the browser/OAuth flow: an older server lacking `/auth/mode`
/// returns 404, and OAuth is the only thing it can do.
async fn probe_auth_mode(url: &str) -> ProbedAuthMode {
    let Ok(http) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    else {
        return ProbedAuthMode::OAuth;
    };

    match moatless_api_client::ApiClient::new(url, http)
        .auth_mode()
        .await
    {
        Ok(resp) if resp.mode == moatless_api_schemas::auth::AuthMode::Password => {
            ProbedAuthMode::Password
        }
        _ => ProbedAuthMode::OAuth,
    }
}

/// Email/password login.
///
/// Composes two existing endpoints — no server-side CLI flow needed: password
/// login for a session cookie, then that cookie mints an `mvk_` API key. Only
/// the key is persisted; the password never touches disk.
async fn login_password(url: &str, login: &LoginEnv) -> Result<(), ClientError> {
    let email = match login.email.clone() {
        Some(value) => value,
        None => dialoguer::Input::<String>::new()
            .with_prompt("Email")
            .interact_text()
            .map_err(|e| ClientError::Api {
                status: 0,
                body: format!("Failed to read email: {e}"),
            })?,
    };
    let password = match login.password.clone() {
        Some(p) => p,
        None => dialoguer::Password::new()
            .with_prompt("Password")
            .interact()
            .map_err(|e| ClientError::Api {
                status: 0,
                body: format!("Failed to read password: {e}"),
            })?,
    };

    let cookie = password_login_cookie(url, &email, &password).await?;

    // Mint a long-lived API key using the freshly minted session.
    let client = MoatClient::new_with_session_cookie(url, &cookie);
    let key = client
        .api()
        .create_api_key(&moatless_api_schemas::auth::CreateApiKeyRequest {
            name: "moat-cli".to_string(),
            scopes: None,
            expires_in_days: None,
        })
        .await?;

    let host = crate::config::host_from_url(url);
    crate::credentials::save_profile(&host, &key.key, &email, Some(url), true).map_err(|e| {
        ClientError::Api {
            status: 500,
            body: format!("Failed to save credentials: {e}"),
        }
    })?;

    println!("Logged in as {email}");
    Ok(())
}

/// Log in with a password and return the raw `name=value` session cookie.
async fn password_login_cookie(
    url: &str,
    email: &str,
    password: &str,
) -> Result<String, ClientError> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ClientError::Api {
            status: 500,
            body: format!("Failed to build HTTP client: {e}"),
        })?;

    let resp = http
        .post(format!("{url}/api/v1/auth/password/login"))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .map_err(|e| ClientError::Api {
            status: 0,
            body: format!("Failed to reach {url}: {e}. Is the backend running?"),
        })?;

    let status = resp.status().as_u16();
    if status == 401 {
        return Err(ClientError::Api {
            status,
            body: "Invalid email or password".to_string(),
        });
    }
    if !resp.status().is_success() {
        return Err(ClientError::Api {
            status,
            body: resp.text().await.unwrap_or_default(),
        });
    }

    extract_session_cookie(&resp).ok_or_else(|| ClientError::Api {
        status: 500,
        body: "Login succeeded but the server set no session cookie".to_string(),
    })
}

/// Pull the `name=value` pair out of the response's `Set-Cookie` header.
fn extract_session_cookie(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|raw| {
            let pair = raw.split(';').next()?.trim();
            pair.starts_with("moatless_session=")
                .then(|| pair.to_string())
        })
}

/// Browser-based login for standalone CLI usage.
///
/// Binds a one-shot local HTTP server, opens the browser to
/// `{api_url}/api/v1/auth/cli/login?port={port}`, and waits for the callback
/// carrying the API key.
async fn login_browser(api_url: &str) -> Result<(), ClientError> {
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

    let login_url = format!("{api_url}/api/v1/auth/cli/login?port={port}");

    println!("Opening browser to log in...");
    println!("If the browser doesn't open, visit: {login_url}");

    if let Err(e) = open::that(&login_url) {
        eprintln!("Failed to open browser: {e}");
        eprintln!("Please open the URL above manually.");
    }

    // Wait for callback with 120s timeout
    let result = tokio::time::timeout(std::time::Duration::from_mins(2), async {
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
            let host = crate::config::host_from_url(api_url);
            // Store the full URL on the profile so future `moat` invocations
            // pick up the right host without needing `MOATLESS_API_URL`.
            crate::credentials::save_profile(&host, &api_key, &login, Some(api_url), true)
                .map_err(|e| ClientError::Api {
                    status: 500,
                    body: format!("Failed to save credentials: {e}"),
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

/// Ensure we have valid auth. In sandbox mode, exchanges token for session. In
/// standalone mode, checks for API key and prompts login if missing.
pub async fn ensure_auth(client: &mut MoatClient, config: &Config) -> Result<(), ClientError> {
    match &config.auth_mode {
        AuthMode::Sandbox { .. } => ensure_session_sandbox(client, config).await,
        AuthMode::Standalone { api_key } => {
            if api_key.is_none() {
                // No API key — try an unauthenticated probe to see if the
                // server requires auth.  If it returns 401 we surface
                // the "not logged in" error; otherwise let it through.
                match client.raw_get("/api/v1/repositories").await {
                    Ok(_) => Ok(()),
                    Err(ClientError::Unauthorized) => Err(ClientError::NotLoggedIn),
                    Err(_) => Ok(()), // network / other error — let the actual command handle it
                }
            } else {
                Ok(())
            }
        }
    }
}

/// Remove the current host's saved credentials.
pub fn logout(cfg: &Config) {
    let host = cfg.host();
    match crate::credentials::delete_credential(&host) {
        Ok(true) => println!("Logged out from {host}."),
        Ok(false) => println!("No credentials found for {host}."),
        Err(e) => eprintln!("Error: {e}"),
    }
}
