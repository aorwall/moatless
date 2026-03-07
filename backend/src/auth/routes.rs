//! Auth API route handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Json;
use rand::Rng;
use tracing::{debug, info, warn};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::config::{AuthProvider, Config};
use super::sessions::SessionRepository;
use super::api_keys::ApiKeyRepository;
use super::users::UserRepository;
use super::provider_tokens::ProviderTokenAuthRepository;
use crate::db::DbPool;
use crate::sandbox::schemas::ErrorBody;

use super::api_key;
use super::cache::AuthCache;
use super::cookie;
use super::error::AuthError;
use super::jwt::{self, SessionClaims};
use super::oauth::provider::OAuthProvider;
use super::session;
use super::schemas::{
    ApiKeyCreateResponse, ApiKeyListItem, CreateApiKeyRequest, MessageResponse,
    OAuthCallbackQuery, SessionResponse, UpdateUserRequest, UserInfo, UserListItem,
    UserListResponse, format_datetime, format_datetime_opt,
};

/// Type-safe container for configured OAuth providers.
#[derive(Clone, Default)]
pub struct OAuthProviders {
    pub github: Option<Arc<dyn OAuthProvider>>,
    pub auth0: Option<Arc<dyn OAuthProvider>>,
}

impl OAuthProviders {
    /// Build providers from application config.
    ///
    /// Each provider is only created when its required env vars are set.
    pub fn from_config(config: &Config) -> Self {
        let github = super::oauth::github::GitHubOAuthProvider::from_config(config).map(|p| {
            tracing::info!("GitHub OAuth provider configured");
            Arc::new(p) as Arc<dyn OAuthProvider>
        });
        let auth0 = super::oauth::auth0::Auth0OAuthProvider::from_config(config).map(|p| {
            tracing::info!("Auth0 OAuth provider configured");
            Arc::new(p) as Arc<dyn OAuthProvider>
        });
        Self { github, auth0 }
    }

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
    pub sessions: Arc<dyn SessionRepository>,
    pub api_keys: Arc<dyn ApiKeyRepository>,
    pub users: Arc<dyn UserRepository>,
    pub provider_tokens: Arc<dyn ProviderTokenAuthRepository>,
    pub teams: Option<Arc<dyn crate::team::dao::TeamRepository>>,
}

impl AuthState {
    /// Build auth state with Postgres-backed repositories.
    pub fn new(pool: DbPool, config: Arc<Config>) -> Self {
        use super::api_keys::PgApiKeyRepository;
        use super::provider_tokens::PgProviderTokenAuthRepository;
        use super::sessions::PgSessionRepository;
        use super::users::PgUserRepository;

        let providers = OAuthProviders::from_config(&config);
        let cache = AuthCache::new(config.auth.cache_ttl_secs);
        let teams: Option<Arc<dyn crate::team::dao::TeamRepository>> =
            Some(Arc::new(crate::team::dao::PgTeamRepository(pool.clone())));
        Self {
            sessions: Arc::new(PgSessionRepository(pool.clone())),
            api_keys: Arc::new(PgApiKeyRepository(pool.clone())),
            users: Arc::new(PgUserRepository(pool.clone())),
            provider_tokens: Arc::new(PgProviderTokenAuthRepository(pool.clone())),
            providers: Arc::new(providers),
            cache,
            teams,
            db: pool,
            config,
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "auth", description = "Authentication and session management"),
        (name = "api-keys", description = "API key management"),
        (name = "users", description = "User listing"),
    ),
    paths(
        verify_handler,
        me_handler,
        update_me_handler,
        logout_handler,
        exchange_api_key_handler,
        create_api_key_handler,
        list_api_keys_handler,
        revoke_api_key_handler,
        list_users_handler,
    )
)]
pub struct AuthApiDoc;

/// Create the auth router with all endpoints.
pub fn auth_router(state: AuthState) -> (axum::Router, utoipa::openapi::OpenApi) {
    // Build OpenAPI-documented routes
    let (openapi_router, api) = OpenApiRouter::with_openapi(AuthApiDoc::openapi())
        .routes(routes!(verify_handler))
        .routes(routes!(me_handler, update_me_handler))
        .routes(routes!(logout_handler))
        .routes(routes!(exchange_api_key_handler))
        .routes(routes!(create_api_key_handler, list_api_keys_handler))
        .routes(routes!(revoke_api_key_handler))
        .routes(routes!(list_users_handler))
        .with_state(state.clone())
        .split_for_parts();

    // Build a separate plain Router for OAuth redirect routes (not in OpenAPI — browser-only flows)
    let oauth_router = axum::Router::new()
        .route("/api/v1/auth/login", get(login_handler))
        .route("/api/v1/auth/github", get(oauth_init_handler))
        .route("/api/v1/auth/github/callback", get(github_callback_handler))
        .route("/api/v1/auth/auth0", get(auth0_init_handler))
        .route("/api/v1/auth/auth0/callback", get(auth0_callback_handler))
        .route("/api/v1/auth/logout", get(logout_get_handler))
        .with_state(state);

    // Merge both into a single Router
    let router: axum::Router = Into::<axum::Router>::into(openapi_router).merge(oauth_router);

    (router, api)
}

// ============================================================================
// Helper: resolve JWT secret from config
// ============================================================================

/// Resolve the session JWT secret, returning an error if none is available.
fn require_jwt_secret(config: &Config) -> Result<&str, AuthError> {
    jwt::get_jwt_secret(config)
        .ok_or_else(|| AuthError::Config("No session JWT secret configured".to_string()))
}

// ============================================================================
// Helper: authenticate from session cookie (JWT)
// ============================================================================

/// Try to authenticate from session cookie, returning the JWT claims.
fn authenticate_from_cookie(
    config: &Config,
    headers: &HeaderMap,
) -> Result<SessionClaims, AuthError> {
    let secret = require_jwt_secret(config)?;
    let token = cookie::get_cookie_value(headers, &config.auth.session_cookie_name)
        .ok_or(AuthError::Unauthorized)?;
    jwt::verify_session_jwt(&token, secret).map_err(|_| AuthError::SessionExpired)
}

// ============================================================================
// GET /api/v1/auth/verify — Traefik ForwardAuth endpoint
// ============================================================================

/// Verify authentication via session cookie or API key (Traefik ForwardAuth).
#[utoipa::path(
    get,
    path = "/api/v1/auth/verify",
    tag = "auth",
    responses(
        (status = 200, description = "Authentication valid"),
        (status = 401, description = "Authentication required", body = ErrorBody)
    )
)]
async fn verify_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    // Dev mode: auth disabled
    if state.config.auth.provider.is_none() {
        return Ok((StatusCode::OK, "OK").into_response());
    }

    let fwd_method = headers
        .get("x-forwarded-method")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");
    let fwd_uri = headers
        .get("x-forwarded-uri")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");

    // Try API key (Authorization: Bearer mvk_xxx)
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if auth_header.starts_with("Bearer mvk_") {
            let key = &auth_header[7..]; // strip "Bearer "
            if let Some(payload) = api_key::validate_key_cached(state.api_keys.clone(), state.users.as_ref(), key, &state.cache).await? {
                debug!(user_id = %payload.user_id, "Auth verify: API key valid");
                return Ok(verify_success_response(&payload.user_id, &payload.user_login, payload.role.as_str(), &payload.provider, payload.group_id.as_deref()));
            }
            warn!(method = fwd_method, uri = fwd_uri, "Auth verify: invalid API key");
            return Err(AuthError::InvalidApiKey);
        }
    }

    // Try session cookies (JWT) — there may be duplicates when COOKIE_DOMAIN changed
    let jwt_secret = require_jwt_secret(&state.config)?;
    let tokens = cookie::get_all_cookie_values(&headers, &state.config.auth.session_cookie_name);
    if tokens.is_empty() {
        warn!(method = fwd_method, uri = fwd_uri, "Auth verify: no session cookie found");
        return Err(AuthError::Unauthorized);
    }

    for token in &tokens {
        match jwt::verify_session_jwt(token, jwt_secret) {
            Ok(claims) => {
                if tokens.len() > 1 {
                    warn!(
                        count = tokens.len(),
                        "Auth verify: multiple session cookies found, using valid one"
                    );
                }
                debug!(user_id = %claims.sub, "Auth verify: session JWT valid");
                return Ok(verify_success_response(
                    &claims.sub,
                    &claims.login,
                    &claims.role,
                    &claims.provider,
                    claims.group_id.as_deref(),
                ));
            }
            Err(_) => {
                // This cookie value is invalid/expired — try the next one
                continue;
            }
        }
    }

    warn!(
        count = tokens.len(),
        "Auth verify: all session cookies invalid/expired"
    );
    Err(AuthError::SessionExpired)
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

/// Get current user session information.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth",
    responses(
        (status = 200, description = "Current session info", body = SessionResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
async fn me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthError> {
    // Dev mode
    if state.config.auth.provider.is_none() {
        return Ok(Json(SessionResponse {
            user: None,
            expires_at: None,
        }));
    }

    let claims = authenticate_from_cookie(&state.config, &headers)?;

    // Fetch fresh user data from DB (email/name/avatar may have changed)
    let user = state.users.get_user(&claims.sub).await?;
    let Some(user) = user else {
        return Err(AuthError::SessionExpired);
    };

    let role = claims.user_role();
    let expires_at = chrono::DateTime::from_timestamp(claims.exp as i64, 0);

    Ok(Json(SessionResponse {
        user: Some(UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: claims.provider,
            role,
        }),
        expires_at: expires_at.map(format_datetime),
    }))
}

// ============================================================================
// PATCH /api/v1/auth/me — update current user profile
// ============================================================================

/// Update current user's profile (name, email).
#[utoipa::path(
    patch,
    path = "/api/v1/auth/me",
    tag = "auth",
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "Profile updated", body = SessionResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
async fn update_me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<SessionResponse>, AuthError> {
    let claims = authenticate_from_cookie(&state.config, &headers)?;

    let role = claims.user_role();

    // Update the users table
    let user = state
        .users
        .update_user_profile(
            &claims.sub,
            body.name.clone(),
            body.email.clone(),
        )
        .await?
        .ok_or_else(|| AuthError::NotFound("User not found".to_string()))?;

    let expires_at = chrono::DateTime::from_timestamp(claims.exp as i64, 0);

    Ok(Json(SessionResponse {
        user: Some(UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: claims.provider,
            role,
        }),
        expires_at: expires_at.map(format_datetime),
    }))
}

// ============================================================================
// GET /api/v1/auth/login — redirect to configured provider
// ============================================================================

async fn login_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    match state.config.auth.provider {
        None => Ok(Redirect::to(&state.config.auth.frontend_url).into_response()),
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

    // Resolve JWT secret
    let jwt_secret = require_jwt_secret(&state.config)?;

    // Resolve team memberships
    let team_ids = if let Some(ref teams_repo) = state.teams {
        resolve_team_ids_on_login(
            teams_repo.as_ref(),
            &user_info.id,
            group_id.as_deref(),
            &state.config,
        )
        .await
    } else {
        vec![]
    };

    // Create session (returns JWT)
    let provider_str = provider_id.to_string();
    let jwt_token = session::create_session(
        state.users.as_ref(),
        state.sessions.as_ref(),
        state.provider_tokens.as_ref(),
        &provider_str,
        &tokens,
        &user_info,
        group_id.as_deref(),
        &team_ids,
        state.config.auth.session_max_age_secs,
        jwt_secret,
    )
    .await?;

    // Build response: set session cookie, clear state cookie, redirect to frontend
    let session_cookie =
        cookie::build_session_cookie(&state.config, &jwt_token, state.config.auth.session_max_age_secs);
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
        [("Location", state.config.auth.frontend_url.as_str())],
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
    if state.config.auth.provider.is_none() {
        return Ok(Json(MessageResponse {
            message: "Logged out successfully".to_string(),
        })
        .into_response());
    }

    // Decode JWT to determine provider (for Auth0 redirect)
    let mut session_provider: Option<AuthProvider> = None;
    if let Some(token) = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name) {
        if let Some(secret) = jwt::get_jwt_secret(&state.config) {
            if let Ok(claims) = jwt::verify_session_jwt(&token, secret) {
                session_provider = claims.provider.parse().ok();
            }
        }
    }

    let clear_cookie = cookie::build_clear_session_cookie(&state.config);
    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = clear_cookie.parse() {
        resp_headers.insert(header::SET_COOKIE, v);
    }

    // Auth0 requires server-side logout
    if let Some(pid) = session_provider {
        if let Some(provider) = state.providers.get(pid) {
            if let Some(logout_url) = provider.generate_logout_url(&state.config.auth.frontend_url) {
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

/// Log out the current user session.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses(
        (status = 200, description = "Logged out successfully", body = MessageResponse),
        (status = 302, description = "Redirect to provider logout")
    )
)]
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
// POST /api/v1/auth/api-key/session — exchange API key for session cookie
// ============================================================================

/// Exchange a valid API key for a session cookie (for local dev auth).
#[utoipa::path(
    post,
    path = "/api/v1/auth/api-key/session",
    tag = "auth",
    responses(
        (status = 200, description = "Session created", body = SessionResponse),
        (status = 401, description = "Invalid API key", body = ErrorBody)
    )
)]
async fn exchange_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Unauthorized)?;

    if !auth_header.starts_with("Bearer mvk_") {
        return Err(AuthError::InvalidApiKey);
    }

    let key = &auth_header[7..]; // strip "Bearer "
    let payload = api_key::validate_key(state.api_keys.clone(), state.users.as_ref(), key)
        .await?
        .ok_or(AuthError::InvalidApiKey)?;

    let user = state.users.get_user(&payload.user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    let jwt_secret = require_jwt_secret(&state.config)?;

    let team_ids = if let Some(ref teams_repo) = state.teams {
        teams_repo.find_team_ids_for_user(&user.id).await.unwrap_or_default()
    } else {
        vec![]
    };

    let jwt_token = session::create_session_from_user(
        state.sessions.as_ref(),
        &user,
        payload.group_id.as_deref(),
        &team_ids,
        state.config.auth.session_max_age_secs,
        jwt_secret,
    )
    .await?;

    let session_cookie = cookie::build_session_cookie(
        &state.config,
        &jwt_token,
        state.config.auth.session_max_age_secs,
    );

    // Build response from user data + claims
    let body = SessionResponse {
        user: Some(UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: "api-key".to_string(),
            role: user.role,
        }),
        expires_at: None,
    };

    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = session_cookie.parse() {
        resp_headers.insert(header::SET_COOKIE, v);
    }

    Ok((resp_headers, Json(body)).into_response())
}

// ============================================================================
// API key management
// ============================================================================

/// Create a new API key for the authenticated user.
#[utoipa::path(
    post,
    path = "/api/v1/auth/api-keys",
    tag = "api-keys",
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "API key created", body = ApiKeyCreateResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
async fn create_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreateResponse>), AuthError> {
    let claims = authenticate_from_cookie(&state.config, &headers)?;

    let expires_at = body.expires_in_days.map(|days| {
        chrono::Utc::now() + chrono::Duration::days(days as i64)
    });

    let result = api_key::generate_key(
        state.api_keys.as_ref(),
        &claims.sub,
        &body.name,
        body.scopes,
        expires_at,
        claims.group_id.as_deref(),
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

/// List all API keys for the authenticated user.
#[utoipa::path(
    get,
    path = "/api/v1/auth/api-keys",
    tag = "api-keys",
    responses(
        (status = 200, description = "List of API keys", body = Vec<ApiKeyListItem>),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
async fn list_api_keys_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeyListItem>>, AuthError> {
    let claims = authenticate_from_cookie(&state.config, &headers)?;
    let keys = api_key::list_keys(state.api_keys.as_ref(), &claims.sub).await?;

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

/// Revoke an API key by ID.
#[utoipa::path(
    delete,
    path = "/api/v1/auth/api-keys/{key_id}",
    tag = "api-keys",
    params(
        ("key_id" = String, Path, description = "API key identifier")
    ),
    responses(
        (status = 200, description = "API key revoked", body = MessageResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 404, description = "API key not found", body = ErrorBody)
    )
)]
async fn revoke_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<MessageResponse>, AuthError> {
    let claims = authenticate_from_cookie(&state.config, &headers)?;
    let result = api_key::revoke_key(state.api_keys.as_ref(), &key_id, &claims.sub).await?;

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

/// List all registered users.
#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    responses(
        (status = 200, description = "List of users", body = UserListResponse),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
async fn list_users_handler(
    State(state): State<AuthState>,
) -> Result<Json<UserListResponse>, AuthError> {
    let users = state.users.list_users().await?;

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

// ============================================================================
// Team resolution helpers
// ============================================================================

/// Resolve team IDs for a user on login.
///
/// For Auth0 users with a group claim, upserts the group as a team and adds membership.
/// For all users, checks for default team assignment if the user has no teams.
async fn resolve_team_ids_on_login(
    teams: &dyn crate::team::dao::TeamRepository,
    user_id: &str,
    group_id: Option<&str>,
    config: &Config,
) -> Vec<String> {
    // If Auth0 group claim is present, upsert team and add membership
    if let Some(group) = group_id {
        let slug = group.to_lowercase().replace(' ', "-");
        match teams.upsert_by_external(group, "auth0", group, &slug).await {
            Ok(team) => {
                if let Err(e) = teams.add_member(&team.id, user_id, "member").await {
                    warn!(error = %e, user_id, team_id = %team.id, "Failed to add user to Auth0 team");
                }
            }
            Err(e) => {
                warn!(error = %e, user_id, group = group, "Failed to upsert Auth0 team");
            }
        }
    }

    // Fetch current team memberships
    let mut team_ids = teams.find_team_ids_for_user(user_id).await.unwrap_or_default();

    // If no teams and a default team is configured, add user to it
    if team_ids.is_empty() {
        if let Some(ref slug) = config.auth.default_team_slug {
            if let Ok(Some(default_team)) = teams.find_by_slug(slug).await {
                if let Err(e) = teams.add_member(&default_team.id, user_id, "member").await {
                    warn!(error = %e, user_id, "Failed to add user to default team");
                } else {
                    team_ids.push(default_team.id);
                }
            }
        }
    }

    team_ids
}
