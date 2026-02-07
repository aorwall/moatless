//! Auth API route handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::Rng;
use tracing::{error, info, warn};

use crate::config::{AuthProvider, Config};
use crate::db::sessions::SessionRow;
use crate::db::DbPool;

use super::api_key;
use super::cache::AuthCache;
use super::cookie;
use super::error::AuthError;
use super::oauth::provider::OAuthProvider;
use super::session;
use super::types::{
    ApiKeyCreateResponse, ApiKeyListItem, CreateApiKeyRequest, MessageResponse,
    OAuthCallbackQuery, SessionResponse, UserInfo, UserListItem, UserListResponse,
    format_datetime, format_datetime_opt,
};

/// Type-safe container for configured OAuth providers.
#[derive(Clone, Default)]
pub struct OAuthProviders {
    pub github: Option<Arc<dyn OAuthProvider>>,
    pub auth0: Option<Arc<dyn OAuthProvider>>,
}

impl OAuthProviders {
    /// Look up a provider by AuthProvider enum.
    pub fn get(&self, provider: AuthProvider) -> Option<&Arc<dyn OAuthProvider>> {
        match provider {
            AuthProvider::GitHub => self.github.as_ref(),
            AuthProvider::Auth0 => self.auth0.as_ref(),
        }
    }
}

/// Shared state for auth routes.
#[derive(Clone)]
pub struct AuthState {
    pub db: DbPool,
    pub config: Arc<Config>,
    pub providers: Arc<OAuthProviders>,
    pub cache: AuthCache,
}

/// Create the auth router with all endpoints.
pub fn auth_router(state: AuthState) -> Router {
    Router::new()
        // Auth endpoints
        .route("/api/v1/auth/verify", get(verify_handler))
        .route("/api/v1/auth/me", get(me_handler))
        .route("/api/v1/auth/login", get(login_handler))
        .route("/api/v1/auth/github", get(oauth_init_handler))
        .route("/api/v1/auth/github/callback", get(github_callback_handler))
        .route("/api/v1/auth/auth0", get(auth0_init_handler))
        .route("/api/v1/auth/auth0/callback", get(auth0_callback_handler))
        .route("/api/v1/auth/logout", post(logout_handler).get(logout_get_handler))
        // API key endpoints
        .route(
            "/api/v1/auth/api-keys",
            post(create_api_key_handler).get(list_api_keys_handler),
        )
        .route(
            "/api/v1/auth/api-keys/{key_id}",
            delete(revoke_api_key_handler),
        )
        // User endpoints
        .route("/api/v1/users", get(list_users_handler))
        .with_state(state)
}

// ============================================================================
// Helper: extract session from cookie or API key for auth-required endpoints
// ============================================================================

/// Try to authenticate from session cookie, returning the session row.
async fn authenticate_from_cookie(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<SessionRow, AuthError> {
    let session_id = cookie::get_cookie_value(headers, &state.config.session_cookie_name)
        .ok_or(AuthError::Unauthorized)?;
    session::validate_session(&state.db, &session_id)
        .await?
        .ok_or(AuthError::SessionExpired)
}

// ============================================================================
// GET /api/v1/auth/verify — Traefik ForwardAuth endpoint
// ============================================================================

async fn verify_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    // Dev mode: auth disabled
    if state.config.auth_provider.is_none() {
        return Ok((StatusCode::OK, "OK").into_response());
    }

    // Try API key (Authorization: Bearer mvk_xxx)
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if auth_header.starts_with("Bearer mvk_") {
            let key = &auth_header[7..]; // strip "Bearer "
            match api_key::validate_key_cached(&state.db, key, &state.cache).await? {
                Some(payload) => {
                    return Ok(verify_success_response(&payload.user_id, &payload.user_login, &payload.role, &payload.provider, payload.group_id.as_deref()));
                }
                None => {
                    return Err(AuthError::InvalidApiKey);
                }
            }
        }
    }

    // Try session cookie
    let session_id = cookie::get_cookie_value(&headers, &state.config.session_cookie_name);
    let Some(session_id) = session_id else {
        return Err(AuthError::Unauthorized);
    };

    let payload = session::validate_session_cached(&state.db, &session_id, &state.cache).await?;
    let Some(payload) = payload else {
        return Err(AuthError::SessionExpired);
    };

    Ok(verify_success_response(
        &payload.user_id,
        &payload.user_login,
        &payload.role,
        &payload.provider,
        payload.group_id.as_deref(),
    ))
}

/// Build a 200 response with user info headers for ForwardAuth.
fn verify_success_response(
    user_id: &str,
    user_login: &str,
    role: &str,
    provider: &str,
    group_id: Option<&str>,
) -> Response {
    let mut headers = HeaderMap::new();
    // These headers will be forwarded by Traefik to the backend
    if let Ok(v) = user_id.parse() {
        headers.insert("X-User-Id", v);
    }
    if let Ok(v) = user_login.parse() {
        headers.insert("X-User-Login", v);
    }
    if let Ok(v) = role.parse() {
        headers.insert("X-User-Role", v);
    }
    if let Ok(v) = provider.parse() {
        headers.insert("X-User-Provider", v);
    }
    if let Some(gid) = group_id {
        if let Ok(v) = gid.parse() {
            headers.insert("X-Group-Id", v);
        }
    }
    (StatusCode::OK, headers, "OK").into_response()
}

// ============================================================================
// GET /api/v1/auth/me — current user session
// ============================================================================

async fn me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthError> {
    // Dev mode
    if state.config.auth_provider.is_none() {
        return Ok(Json(SessionResponse {
            user: None,
            expires_at: None,
        }));
    }

    let session_id = cookie::get_cookie_value(&headers, &state.config.session_cookie_name);
    let Some(session_id) = session_id else {
        return Err(AuthError::Unauthorized);
    };

    let session = session::validate_session(&state.db, &session_id).await?;
    let Some(session) = session else {
        return Err(AuthError::SessionExpired);
    };

    Ok(Json(session_to_response(&session)))
}

fn session_to_response(session: &SessionRow) -> SessionResponse {
    SessionResponse {
        user: Some(UserInfo {
            id: session.user_id.clone(),
            login: session.user_login.clone(),
            email: session.user_email.clone(),
            name: session.user_name.clone(),
            avatar_url: session.avatar_url.clone(),
            provider: session.provider.clone(),
            role: session.role.clone(),
        }),
        expires_at: Some(format_datetime(session.expires_at)),
    }
}

// ============================================================================
// GET /api/v1/auth/login — redirect to configured provider
// ============================================================================

async fn login_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    match state.config.auth_provider {
        None => Ok(Redirect::to(&state.config.frontend_url).into_response()),
        Some(AuthProvider::GitHub) => Ok(Redirect::to("/api/v1/auth/github").into_response()),
        Some(AuthProvider::Auth0) => Ok(Redirect::to("/api/v1/auth/auth0").into_response()),
    }
}

// ============================================================================
// OAuth init + callback helpers
// ============================================================================

fn generate_csrf_state() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

/// Initiate OAuth flow for a given provider.
async fn initiate_oauth(state: &AuthState, provider_id: AuthProvider) -> Result<Response, AuthError> {
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

    Ok((StatusCode::FOUND, headers, [("Location", auth_url.as_str())]).into_response())
}

/// Handle OAuth callback for a given provider.
async fn handle_oauth_callback(
    state: &AuthState,
    provider_id: AuthProvider,
    code: &str,
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

    // Exchange code for tokens
    let tokens = provider.exchange_code_for_tokens(code).await?;

    // Get user info
    let user_info = provider.get_user_info(&tokens).await?;

    // Extract group (optional)
    let group_id = provider.extract_user_group(&tokens).await;

    // Create session
    let provider_str = provider_id.to_string();
    let session_id = session::create_session(
        &state.db,
        &provider_str,
        &tokens,
        &user_info,
        group_id.as_deref(),
        state.config.session_max_age_secs,
    )
    .await?;

    // Build response: set session cookie, clear state cookie, redirect to frontend
    let session_cookie =
        cookie::build_session_cookie(&state.config, &session_id, state.config.session_max_age_secs);
    let clear_state_cookie = cookie::build_clear_oauth_state_cookie(&state.config);

    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = session_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = clear_state_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }

    Ok((
        StatusCode::FOUND,
        resp_headers,
        [("Location", state.config.frontend_url.as_str())],
    )
        .into_response())
}

// ============================================================================
// GitHub OAuth
// ============================================================================

async fn oauth_init_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::GitHub).await
}

async fn github_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    handle_oauth_callback(&state, AuthProvider::GitHub, &query.code, query.state.as_deref(), &headers).await
}

// ============================================================================
// Auth0 OAuth
// ============================================================================

async fn auth0_init_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::Auth0).await
}

async fn auth0_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    handle_oauth_callback(&state, AuthProvider::Auth0, &query.code, query.state.as_deref(), &headers).await
}

// ============================================================================
// Logout
// ============================================================================

async fn do_logout(state: &AuthState, headers: &HeaderMap) -> Result<Response, AuthError> {
    // Dev mode
    if state.config.auth_provider.is_none() {
        return Ok(Json(MessageResponse {
            message: "Logged out successfully".to_string(),
        })
        .into_response());
    }

    let session_id = cookie::get_cookie_value(headers, &state.config.session_cookie_name);

    // Determine provider before deleting session
    let mut session_provider: Option<AuthProvider> = None;
    if let Some(ref sid) = session_id {
        if let Ok(Some(session)) = session::validate_session(&state.db, sid).await {
            session_provider = AuthProvider::from_str_opt(&session.provider);
        }
        // Delete session regardless
        if let Err(e) = session::delete_session_by_id(&state.db, sid).await {
            error!(error = %e, "Failed to delete session during logout");
        }
        // Invalidate cache
        state.cache.invalidate_session(sid).await;
    }

    let clear_cookie = cookie::build_clear_session_cookie(&state.config);
    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = clear_cookie.parse() {
        resp_headers.insert(header::SET_COOKIE, v);
    }

    // Auth0 requires server-side logout
    if let Some(pid) = session_provider {
        if let Some(provider) = state.providers.get(pid) {
            if let Some(logout_url) = provider.generate_logout_url(&state.config.frontend_url) {
                info!(provider = %pid, "Redirecting to provider logout");
                return Ok((
                    StatusCode::FOUND,
                    resp_headers,
                    [("Location", logout_url.as_str())],
                )
                    .into_response());
            }
        }
    }

    Ok((
        resp_headers,
        Json(MessageResponse {
            message: "Logged out successfully".to_string(),
        }),
    )
        .into_response())
}

async fn logout_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    do_logout(&state, &headers).await
}

async fn logout_get_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    do_logout(&state, &headers).await
}

// ============================================================================
// API key management
// ============================================================================

async fn create_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreateResponse>), AuthError> {
    let session = authenticate_from_cookie(&state, &headers).await?;

    let expires_at = body.expires_in_days.map(|days| {
        chrono::Utc::now().naive_utc() + chrono::Duration::days(days as i64)
    });

    let result = api_key::generate_key(
        &state.db,
        &session.user_id,
        &body.name,
        body.scopes,
        expires_at,
        session.group_id.as_deref(),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreateResponse {
            id: result.id,
            key: result.key,
            name: result.name,
            key_prefix: result.key_prefix,
            scopes: result.scopes,
            expires_at: format_datetime_opt(result.expires_at),
            created_at: format_datetime(result.created_at),
        }),
    ))
}

async fn list_api_keys_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeyListItem>>, AuthError> {
    let session = authenticate_from_cookie(&state, &headers).await?;
    let keys = api_key::list_keys(&state.db, &session.user_id).await?;

    let items: Vec<ApiKeyListItem> = keys
        .iter()
        .map(|k| ApiKeyListItem {
            id: k.id.clone(),
            name: k.name.clone(),
            key_prefix: k.key_prefix.clone(),
            scopes: k
                .scopes
                .as_ref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()),
            last_used_at: format_datetime_opt(k.last_used_at),
            expires_at: format_datetime_opt(k.expires_at),
            created_at: format_datetime(k.created_at),
        })
        .collect();

    Ok(Json(items))
}

async fn revoke_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<MessageResponse>, AuthError> {
    let session = authenticate_from_cookie(&state, &headers).await?;
    let result = api_key::revoke_key(&state.db, &key_id, &session.user_id).await?;

    match result {
        Some(key_hash) => {
            state.cache.invalidate_api_key(&key_hash).await;
            Ok(Json(MessageResponse {
                message: "API key revoked successfully".to_string(),
            }))
        }
        None => Err(AuthError::NotFound(
            "API key not found or not owned by user".to_string(),
        )),
    }
}

// ============================================================================
// User listing
// ============================================================================

async fn list_users_handler(
    State(state): State<AuthState>,
) -> Result<Json<UserListResponse>, AuthError> {
    let users = crate::db::users::list_users(&state.db).await?;

    let items: Vec<UserListItem> = users
        .iter()
        .map(|u| UserListItem {
            id: u.id.clone(),
            login: u.login.clone(),
            name: u.name.clone(),
            avatar_url: u.avatar_url.clone(),
        })
        .collect();

    Ok(Json(UserListResponse { users: items }))
}
