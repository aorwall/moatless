//! Sign-in entry points: the mode a deployment advertises, and the browser and
//! CLI redirects into it.

use super::oauth::generate_csrf_state;
use crate::auth::cookie;
use crate::auth::error::AuthError;
use crate::auth::schemas::{AuthMode, AuthModeResponse, CliLoginQuery};
use crate::auth::state::AuthState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use moatless_types::auth::AuthProvider;

/// Report how this deployment authenticates users.
///
/// Public by design: the login page is anonymous, so this cannot live on
/// `/auth/me` (which 401s without a session).
#[utoipa::path(
    get,
    path = "/api/v1/auth/mode",
    tag = "auth",
    responses((status = 200, description = "Active auth mode", body = AuthModeResponse))
)]
pub(in crate::auth) async fn auth_mode_handler(
    State(state): State<AuthState>,
) -> Json<AuthModeResponse> {
    let provider = state.config.auth.provider;
    let (mode, provider_name, login_url) = match provider {
        AuthProvider::Local => (AuthMode::Password, None, None),
        AuthProvider::GitHubApp => (
            AuthMode::Oauth,
            Some("GitHub".to_string()),
            Some("/api/v1/auth/github-app".to_string()),
        ),
        AuthProvider::Auth0 => (
            AuthMode::Oauth,
            Some("Auth0".to_string()),
            Some("/api/v1/auth/auth0".to_string()),
        ),
        AuthProvider::Oidc => (
            AuthMode::Oauth,
            state
                .config
                .auth
                .oidc
                .as_ref()
                .map(|provider| provider.name.clone()),
            state
                .config
                .auth
                .oidc
                .as_ref()
                .map(|provider| format!("/api/v1/auth/oidc/{}/start", provider.provider_id)),
        ),
    };
    Json(AuthModeResponse {
        mode,
        provider: provider.to_string(),
        provider_name,
        login_url,
    })
}

pub(in crate::auth) async fn login_handler(
    State(state): State<AuthState>,
) -> Result<Response, AuthError> {
    match state.config.auth.provider {
        // Local password auth has no OAuth redirect — send the browser to the
        // frontend login form.
        AuthProvider::Local => {
            Ok(Redirect::to(&format!("{}/login", state.config.auth.frontend_url)).into_response())
        }
        AuthProvider::GitHubApp => Ok(Redirect::to("/api/v1/auth/github-app").into_response()),
        AuthProvider::Auth0 => Ok(Redirect::to("/api/v1/auth/auth0").into_response()),
        AuthProvider::Oidc => {
            let provider = state
                .config
                .auth
                .oidc
                .as_ref()
                .ok_or_else(|| AuthError::Internal("OIDC provider is not configured".into()))?;
            Ok(
                Redirect::to(&format!("/api/v1/auth/oidc/{}/start", provider.provider_id))
                    .into_response(),
            )
        }
    }
}

/// CLI login: initiate OAuth flow with state encoding the CLI callback port.
///
/// The CSRF state is encoded as `{random}:cli:{port}` so that `handle_oauth_callback`
/// can detect the CLI flow and redirect to `http://localhost:{port}/callback`.
pub(in crate::auth) async fn cli_login_handler(
    State(state): State<AuthState>,
    axum::extract::Query(query): axum::extract::Query<CliLoginQuery>,
) -> Result<Response, AuthError> {
    let provider_id = state.config.auth.provider;

    if provider_id == AuthProvider::Oidc {
        let oidc = state
            .oidc
            .as_ref()
            .ok_or_else(|| AuthError::Config("OIDC provider not configured".into()))?;
        let url = oidc.start(None, Some(i32::from(query.port))).await?;
        return Ok(Redirect::to(&url).into_response());
    }

    let provider = state
        .providers
        .get(provider_id)
        .ok_or_else(|| AuthError::Config(format!("Provider {provider_id} not configured")))?;

    // Encode CLI callback port in the CSRF state
    let csrf_base = generate_csrf_state();
    let csrf_state = format!("{csrf_base}:cli:{}", query.port);

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
