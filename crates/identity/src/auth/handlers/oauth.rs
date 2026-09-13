//! The leg every OAuth provider shares: CSRF state, the redirect out, and the
//! callback that lands a session.

use super::session::{authenticate_request, issued_session_headers};
use crate::auth::cookie;
use crate::auth::error::AuthError;
use crate::auth::github_connect;
use crate::auth::state::AuthState;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use moatless_types::auth::AuthProvider;
use rand::RngExt;
use tracing::{info, warn};

pub(super) fn generate_csrf_state() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

/// Initiate OAuth flow for a given provider.
pub(super) async fn initiate_oauth(
    state: &AuthState,
    provider_id: AuthProvider,
) -> Result<Response, AuthError> {
    let provider = state
        .providers
        .get(provider_id)
        .ok_or_else(|| AuthError::Config(format!("Provider {provider_id} not configured")))?;

    let csrf_state = generate_csrf_state();
    let auth_url = provider
        .generate_auth_url(&csrf_state)
        .ok_or_else(|| AuthError::Config("Failed to generate OAuth URL".to_string()))?;

    let state_cookie = cookie::build_oauth_state_cookie(&state.config, &csrf_state);

    let mut headers = HeaderMap::new();
    if let Ok(v) = state_cookie.parse() {
        headers.insert(header::SET_COOKIE, v);
    }

    Ok((
        StatusCode::FOUND,
        headers,
        [("Location", auth_url.as_str())],
    )
        .into_response())
}

/// Handle OAuth callback for a given provider. `code` is absent when the person
/// declined on the provider's consent screen, which is a normal outcome rather
/// than a malformed request.
pub(super) async fn handle_oauth_callback(
    state: &AuthState,
    provider_id: AuthProvider,
    code: Option<&str>,
    query_state: Option<&str>,
    headers: &HeaderMap,
) -> Result<Response, AuthError> {
    // Verify CSRF state — reject if either side is missing
    let stored_state = cookie::get_cookie_value(headers, "oauth_state");
    match (stored_state.as_deref(), query_state) {
        (Some(stored), Some(qs)) if stored == qs => { /* CSRF check passed */ }
        _ => {
            warn!("OAuth state mismatch or missing — possible CSRF attack");
            return Err(AuthError::InvalidState);
        }
    }

    let provider = state
        .providers
        .get(provider_id)
        .ok_or_else(|| AuthError::Config(format!("Provider {provider_id} not configured")))?;

    // Which flow a callback belongs to rides in the CSRF state, so both reach
    // the one callback URL registered on the App and neither can be flipped
    // into the other without breaking the state/cookie match above.
    if let Some(connect_state) = query_state.and_then(github_connect::parse_connect_state) {
        // No session is a viewer that cannot match, not a request error: a
        // consent screen left open past the session's lifetime is the common
        // way to arrive here, and `finish` already returns that person to the
        // page they started from.
        let viewer = authenticate_request(state, headers)
            .await
            .ok()
            .map(|identity| identity.user_id);
        return Ok(
            github_connect::finish(state, provider.as_ref(), viewer, code, &connect_state).await,
        );
    }

    // Check if this is a CLI login flow (state contains ":cli:{port}")
    let cli_port = query_state.and_then(|qs| {
        let parts: Vec<&str> = qs.split(":cli:").collect();
        if parts.len() == 2 {
            parts[1].parse::<u16>().ok()
        } else {
            None
        }
    });

    let present_token = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name);
    let outcome = crate::auth::oauth::login::complete_login(
        state,
        provider.as_ref(),
        provider_id,
        code,
        present_token.as_deref(),
        cli_port.is_some(),
    )
    .await?;
    let (issued, login, cli_key) = match outcome {
        crate::auth::oauth::login::LoginOutcome::OrganizationRequired => {
            let redirect = format!("{}?error=org_required", state.config.auth.frontend_url);
            return Ok((StatusCode::FOUND, [("Location", redirect.as_str())]).into_response());
        }
        crate::auth::oauth::login::LoginOutcome::Completed {
            issued,
            login,
            cli_key,
        } => (issued, login, cli_key),
    };

    if let (Some(port), Some(key)) = (cli_port, cli_key) {
        let login_encoded = urlencoding::encode(&login);
        let key_encoded = urlencoding::encode(&key);
        let redirect_url =
            format!("http://127.0.0.1:{port}/callback?key={key_encoded}&login={login_encoded}");
        info!(user_login = %login, "CLI login successful, redirecting to localhost:{}", port);

        let clear_state_cookie = cookie::build_clear_oauth_state_cookie(&state.config);
        let mut resp_headers = HeaderMap::new();
        if let Ok(v) = clear_state_cookie.parse() {
            resp_headers.append(header::SET_COOKIE, v);
        }

        return Ok((
            StatusCode::FOUND,
            resp_headers,
            [("Location", redirect_url.as_str())],
        )
            .into_response());
    }

    // Normal flow: set session cookie, clear state cookie, redirect to frontend
    let clear_state_cookie = cookie::build_clear_oauth_state_cookie(&state.config);

    let mut resp_headers = issued_session_headers(state, &issued);
    if let Ok(v) = clear_state_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }

    Ok((
        StatusCode::FOUND,
        resp_headers,
        [("Location", state.config.auth.frontend_url.as_str())],
    )
        .into_response())
}
