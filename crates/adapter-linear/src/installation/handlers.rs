//! Browser redirects and OAuth cookie adaptation for Linear installation.
use super::state::Installation;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use moatless_identity::auth::{cookie, error::AuthError, oauth::provider::OAuthProvider};
use std::sync::Arc;
use rand::RngExt;
use tracing::warn;

/// Initiate the Linear workspace install flow.
///
/// Returns 302 to Linear's authorization URL with `actor=app` and the
/// workspace agent scopes. Sets an `oauth_state` cookie for CSRF.
/// Returns 503 when Linear OAuth credentials aren't configured.
pub(super) async fn install(
    State(state): State<Option<Arc<Installation>>>,
) -> Result<Response, AuthError> {
    let state = state.ok_or_else(|| AuthError::Config("Linear OAuth not configured".into()))?;
    let Some(provider) = state.provider.as_ref() else {
        return Err(AuthError::Config(
            "Linear OAuth not configured (set LINEAR_OAUTH_CLIENT_ID, \
             LINEAR_OAUTH_CLIENT_SECRET, LINEAR_OAUTH_CALLBACK_URL)"
                .into(),
        ));
    };

    let bytes: [u8; 16] = rand::rng().random();
    let csrf_state = hex::encode(bytes);
    let auth_url = provider
        .generate_auth_url(&csrf_state)
        .ok_or_else(|| AuthError::Config("Failed to generate Linear OAuth URL".to_string()))?;

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

/// Handle the Linear workspace install callback.
///
/// 1. CSRF check via cookie.
/// 2. Exchange code → workspace token (form-encoded).
/// 3. Fetch viewer + organization to identify the agent + workspace.
/// 4. Upsert `adapter_connections(adapter_kind="linear", external_account_id=org_id)`.
/// 5. Upsert the global `provider_tokens(provider="linear")` row (workspace
///    OAuth tokens are stored globally — the Linear org UUID lives in
///    `adapter_connections.external_account_id` and `provider_tokens.metadata`).
pub(super) async fn callback(
    State(state): State<Option<Arc<Installation>>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<
        moatless_api_schemas::auth::OAuthCallbackQuery,
    >,
) -> Result<Response, AuthError> {
    let state = state.ok_or_else(|| AuthError::Config("Linear OAuth not configured".into()))?;
    state
        .provider
        .as_ref()
        .ok_or_else(|| AuthError::Config("Linear OAuth not configured".into()))?;

    let stored_state = cookie::get_cookie_value(&headers, "oauth_state");
    match (stored_state.as_deref(), query.state.as_deref()) {
        (Some(stored), Some(qs)) if stored == qs => { /* CSRF check passed */ }
        _ => {
            warn!("Linear OAuth state mismatch or missing — possible CSRF attack");
            return Err(AuthError::InvalidState);
        }
    }

    let code = query
        .code
        .as_deref()
        .ok_or_else(|| AuthError::OAuth("Authorization was declined".into()))?;
    let organization_id = state.complete_install(code).await?;

    let clear_state_cookie = cookie::build_clear_oauth_state_cookie(&state.config);
    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = clear_state_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }
    let redirect = format!(
        "{}?linear_installed={}",
        state.config.auth.frontend_url,
        urlencoding::encode(&organization_id)
    );
    Ok((
        StatusCode::FOUND,
        resp_headers,
        [("Location", redirect.as_str())],
    )
        .into_response())
}
