//! Per-provider entry and callback routes: GitHub App, Forgejo, Auth0 and OIDC.

use super::oauth::{generate_csrf_state, handle_oauth_callback, initiate_oauth};
use super::session::{
    authenticate_request, issued_session_headers, request_source, revoke_present_session,
};
use crate::auth::api_key;
use crate::auth::cookie;
use crate::auth::error::AuthError;
use crate::auth::forgejo_connect;
use crate::auth::github_connect;
use crate::auth::schemas::OAuthCallbackQuery;
use crate::auth::sessions as session;
use crate::auth::state::AuthState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use moatless_types::auth::AuthProvider;
use tracing::warn;

pub(in crate::auth) async fn github_app_init_handler(
    State(state): State<AuthState>,
) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::GitHubApp).await
}

#[derive(serde::Deserialize)]
pub(in crate::auth) struct GitHubConnectQuery {
    /// Where to send the browser once the App is authorized. Vetted against the
    /// trusted origins — see
    /// [`crate::auth::connect_return::approved_return_url`].
    return_to: Option<String>,
}

/// Authorize the GitHub App for the caller's existing session.
///
/// Requires a session because the credential is written against it — this
/// establishes GitHub *access*, leaving how the caller signed in alone.
pub(in crate::auth) async fn github_app_connect_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<GitHubConnectQuery>,
) -> Result<Response, AuthError> {
    // Carried into the CSRF state so the callback writes the credential to
    // whoever started the flow, not to whoever happens to be signed in when
    // GitHub redirects back — see [`github_connect::finish`].
    let identity = authenticate_request(&state, &headers).await?;
    let provider = state
        .providers
        .get(AuthProvider::GitHubApp)
        .ok_or_else(|| {
            AuthError::Config(
                "GitHub App OAuth is not configured — set GITHUB_APP_CLIENT_ID, \
                 GITHUB_APP_CLIENT_SECRET and GITHUB_APP_CALLBACK_URL"
                    .to_string(),
            )
        })?
        .clone();
    github_connect::start(
        &state,
        provider.as_ref(),
        &generate_csrf_state(),
        identity.user_id,
        query.return_to.as_deref(),
    )
}

pub(in crate::auth) async fn github_app_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    handle_oauth_callback(
        &state,
        AuthProvider::GitHubApp,
        query.code.as_deref(),
        query.state.as_deref(),
        &headers,
    )
    .await
}

#[derive(serde::Deserialize)]
pub(in crate::auth) struct ForgejoConnectQuery {
    /// Which instance to connect. Forgejo is self-hostable, so it selects the
    /// registered OAuth2 application, the credential row and the secret.
    host: String,
    /// Where to send the browser once the instance is authorized. Vetted
    /// against the trusted origins — see
    /// [`crate::auth::connect_return::approved_return_url`].
    return_to: Option<String>,
}

/// Authorize a Forgejo instance for the caller's existing session.
///
/// Requires a session because the credential is written against it — this
/// establishes Forgejo *access*, leaving how the caller signed in alone.
pub(in crate::auth) async fn forgejo_connect_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<ForgejoConnectQuery>,
) -> Result<Response, AuthError> {
    // Carried into the CSRF state so the callback writes the credential to
    // whoever started the flow, not to whoever happens to be signed in when
    // Forgejo redirects back — see [`forgejo_connect::finish`].
    let identity = authenticate_request(&state, &headers).await?;
    forgejo_connect::start(
        &state,
        &generate_csrf_state(),
        identity.user_id,
        &query.host,
        query.return_to.as_deref(),
    )
    .await
}

pub(in crate::auth) async fn forgejo_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    let stored_state = cookie::get_cookie_value(&headers, "oauth_state");
    match (stored_state.as_deref(), query.state.as_deref()) {
        (Some(stored), Some(qs)) if stored == qs => { /* CSRF check passed */ }
        _ => {
            warn!("Forgejo OAuth state mismatch or missing — possible CSRF attack");
            return Err(AuthError::InvalidState);
        }
    }
    let connect = query
        .state
        .as_deref()
        .and_then(forgejo_connect::parse_connect_state)
        .ok_or(AuthError::InvalidState)?;

    // No session is a viewer that cannot match, not a request error: a consent
    // screen left open past the session's lifetime is the common way to arrive
    // here, and `finish` already returns that person to the page they started
    // from.
    let viewer = authenticate_request(&state, &headers).await.ok();
    Ok(forgejo_connect::finish(&state, viewer.as_ref(), query.code.as_deref(), &connect).await)
}

pub(in crate::auth) async fn auth0_init_handler(
    State(state): State<AuthState>,
) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::Auth0).await
}

pub(in crate::auth) async fn auth0_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    handle_oauth_callback(
        &state,
        AuthProvider::Auth0,
        query.code.as_deref(),
        query.state.as_deref(),
        &headers,
    )
    .await
}

#[derive(serde::Deserialize)]
pub(in crate::auth) struct OidcStartQuery {
    return_to: Option<String>,
    port: Option<i32>,
}

pub(in crate::auth) async fn oidc_start_handler(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OidcStartQuery>,
) -> Result<Response, AuthError> {
    let oidc = state
        .oidc
        .as_ref()
        .filter(|oidc| oidc.provider_id() == provider)
        .ok_or_else(|| AuthError::NotFound("OIDC provider is not configured".into()))?;
    if query
        .port
        .is_some_and(|port| !(1..=u16::MAX as i32).contains(&port))
    {
        return Err(AuthError::BadRequest("invalid CLI callback port".into()));
    }
    if let Some(source) = request_source(&headers) {
        if state.login_throttle.is_locked("oidc_start", source).await? {
            return Err(AuthError::TooManyRequests);
        }
        state
            .login_throttle
            .record_failure("oidc_start", source)
            .await?;
    }
    let url = oidc.start(query.return_to, query.port).await?;
    Ok(Redirect::to(&url).into_response())
}

pub(in crate::auth) async fn oidc_callback_handler(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    let oidc = state
        .oidc
        .as_ref()
        .filter(|oidc| oidc.provider_id() == provider)
        .ok_or_else(|| AuthError::NotFound("OIDC provider is not configured".into()))?;
    let source = request_source(&headers);
    if let Some(source) = source {
        if state
            .login_throttle
            .is_locked("oidc_callback_failure", source)
            .await?
        {
            return Err(AuthError::TooManyRequests);
        }
    }
    let result = match oidc
        .finish(
            query.state.as_deref().ok_or(AuthError::InvalidState)?,
            query
                .code
                .ok_or_else(|| AuthError::OAuth("Authorization was declined".into()))?,
        )
        .await
    {
        Ok(result) => result,
        Err(error) => {
            if let Some(source) = source {
                state
                    .login_throttle
                    .record_failure("oidc_callback_failure", source)
                    .await?;
            }
            return Err(error);
        }
    };
    crate::auth::admin_grant::grant_listed_admin(
        state.users.as_ref(),
        &state.config.auth.admin_user_ids,
        &result.user.id,
        oidc.provider_id(),
        &result.subject,
    )
    .await?;
    if let Some(port) = result.cli_callback_port {
        let key = api_key::generate_key(
            state.api_keys.as_ref(),
            &result.user.id.to_string(),
            "moat-cli",
            None,
            None,
            None,
        )
        .await?;
        let key = urlencoding::encode(&key.key);
        let login = urlencoding::encode(&result.user.login);
        let location = format!("http://127.0.0.1:{port}/callback?key={key}&login={login}");
        return Ok(Redirect::to(&location).into_response());
    }

    revoke_present_session(&state, &headers).await?;
    let issued = session::create_session_from_user_with_provider(
        state.sessions.as_ref(),
        &result.user,
        None,
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
        &format!("oidc:{}", oidc.provider_id()),
    )
    .await?;

    let headers = issued_session_headers(&state, &issued);
    let path = result
        .redirect_after_login
        .unwrap_or_else(|| "/".to_string());
    let location = format!(
        "{}{}",
        state.config.auth.frontend_url.trim_end_matches('/'),
        path
    );
    Ok((StatusCode::FOUND, headers, [("Location", location)]).into_response())
}
