//! Auth API route handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Json;
use rand::Rng;
use tracing::{info, warn};
use urlencoding;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::config::{AuthProvider, Config};
use crate::db::DbPool;
use crate::error::ErrorBody;
use crate::scope::ScopeContext;

use super::api_key;
use super::api_keys::ApiKeyRepository;
use super::cache::AuthCache;
use super::cookie;
use super::error::AuthError;
use super::github_app_service::GitHubAppService;
use super::identity::AuthIdentity;
use super::jwt::{self, SessionClaims};
use super::models::UserRole;
use super::oauth::provider::OAuthProvider;
use super::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
use super::provider_tokens::ProviderTokenAuthRepository;
use moatless_api_schemas::user::{SetProviderIdRequest, UserProviderIdItem};

use super::schemas::{
    format_datetime, format_datetime_opt, ApiKeyCreateResponse, ApiKeyListItem, CliLoginQuery,
    CreateApiKeyRequest, MessageResponse, OAuthCallbackQuery, ProviderConnection, SessionResponse,
    TokenSessionResponse, UpdateUserRequest, UserInfo, UserListItem, UserListResponse,
};
use super::session;
use super::sessions::SessionRepository;
use super::users::UserRepository;

/// Type-safe container for configured OAuth providers.
#[derive(Clone, Default)]
pub struct OAuthProviders {
    pub github: Option<Arc<dyn OAuthProvider>>,
    pub github_app: Option<Arc<dyn OAuthProvider>>,
    pub auth0: Option<Arc<dyn OAuthProvider>>,
    /// Linear workspace install. Not used for user login — issued tokens are
    /// stored at the workspace level (`provider_tokens.team_id = <org uuid>`)
    /// and consumed by the Linear Agents API client.
    pub linear: Option<Arc<super::oauth::linear::LinearOAuthProvider>>,
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
        let github_app =
            super::oauth::github_app::GitHubAppOAuthProvider::from_config(config).map(|p| {
                tracing::info!("GitHub App OAuth provider configured");
                Arc::new(p) as Arc<dyn OAuthProvider>
            });
        if config.auth.provider == Some(AuthProvider::GitHubApp) && github_app.is_none() {
            tracing::warn!(
                "AUTH_PROVIDER=github_app but GitHubAppOAuthProvider failed to initialize \
                 — check GITHUB_APP_CLIENT_ID, GITHUB_APP_CLIENT_SECRET, GITHUB_APP_CALLBACK_URL"
            );
        }
        let auth0 = super::oauth::auth0::Auth0OAuthProvider::from_config(config).map(|p| {
            tracing::info!("Auth0 OAuth provider configured");
            Arc::new(p) as Arc<dyn OAuthProvider>
        });
        let linear = super::oauth::linear::LinearOAuthProvider::from_config(config).map(|p| {
            tracing::info!("Linear OAuth provider configured");
            Arc::new(p)
        });
        Self {
            github,
            github_app,
            auth0,
            linear,
        }
    }

    /// Look up a provider by AuthProvider enum.
    pub fn get(&self, provider: AuthProvider) -> Option<&Arc<dyn OAuthProvider>> {
        match provider {
            AuthProvider::GitHub => self.github.as_ref(),
            AuthProvider::GitHubApp => self.github_app.as_ref(),
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
    /// DAO for credential lookups (full-row reads, refresh persistence).
    /// Separate from `provider_tokens` (which handles upsert/metadata paths).
    pub provider_tokens_dao: Arc<dyn ProviderTokenRepository>,
    /// Optional GitHub App service used to refresh expiring user tokens when
    /// reporting provider connection status on `/me`.
    pub github_app_service: Option<Arc<GitHubAppService>>,
    pub teams: Option<Arc<dyn crate::team::dao::TeamRepository>>,
    pub tasks: Option<Arc<dyn crate::task::dao::TaskRepository>>,
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
        let tasks: Option<Arc<dyn crate::task::dao::TaskRepository>> =
            Some(Arc::new(crate::task::dao::PgTaskRepository(pool.clone())));
        let github_app_service = GitHubAppService::from_config(&config).map(Arc::new);
        Self {
            sessions: Arc::new(PgSessionRepository(pool.clone())),
            api_keys: Arc::new(PgApiKeyRepository(pool.clone())),
            users: Arc::new(PgUserRepository(pool.clone())),
            provider_tokens: Arc::new(PgProviderTokenAuthRepository(pool.clone())),
            provider_tokens_dao: Arc::new(PgProviderTokenRepository(pool.clone())),
            github_app_service,
            providers: Arc::new(providers),
            cache,
            teams,
            tasks,
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
        exchange_token_handler,
        create_api_key_handler,
        list_api_keys_handler,
        revoke_api_key_handler,
        list_users_handler,
        set_user_provider_id_handler,
        delete_user_provider_id_handler,
        list_user_provider_ids_handler,
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
        .routes(routes!(exchange_token_handler))
        .routes(routes!(create_api_key_handler, list_api_keys_handler))
        .routes(routes!(revoke_api_key_handler))
        .routes(routes!(list_users_handler))
        .routes(routes!(
            set_user_provider_id_handler,
            delete_user_provider_id_handler
        ))
        .routes(routes!(list_user_provider_ids_handler))
        .with_state(state.clone())
        .split_for_parts();

    // Build a separate plain Router for OAuth redirect routes (not in OpenAPI — browser-only flows)
    let oauth_router = axum::Router::new()
        .route("/api/v1/auth/login", get(login_handler))
        .route("/api/v1/auth/cli/login", get(cli_login_handler))
        .route("/api/v1/auth/github", get(oauth_init_handler))
        .route("/api/v1/auth/github/callback", get(github_callback_handler))
        .route("/api/v1/auth/github-app", get(github_app_init_handler))
        .route(
            "/api/v1/auth/github-app/callback",
            get(github_app_callback_handler),
        )
        .route("/api/v1/auth/auth0", get(auth0_init_handler))
        .route("/api/v1/auth/auth0/callback", get(auth0_callback_handler))
        .route("/api/v1/auth/linear/install", get(linear_install_handler))
        .route(
            "/api/v1/auth/linear/callback",
            get(linear_callback_handler),
        )
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
// Shared auth helper
// ============================================================================

/// Authenticate from session cookie or API key.
///
/// Used by both `verify_handler` (ForwardAuth) and internal callers.
async fn authenticate_request(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, AuthError> {
    // Try API key (Authorization: Bearer mvk_xxx)
    if let Some(auth_header) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if auth_header.starts_with("Bearer mvk_") {
            let key = &auth_header[7..]; // strip "Bearer "
            if let Some(payload) = api_key::validate_key_cached(
                state.api_keys.clone(),
                state.users.as_ref(),
                key,
                &state.cache,
            )
            .await?
            {
                return Ok(AuthIdentity {
                    user_id: payload.user_id,
                    login: payload.user_login,
                    role: payload.role,
                    provider: payload.provider,
                    group_id: payload.group_id,
                    team_ids: payload.team_ids,
                });
            }
            return Err(AuthError::InvalidApiKey);
        }
    }

    // Try session cookies (JWT)
    let jwt_secret = require_jwt_secret(&state.config)?;
    let tokens = cookie::get_all_cookie_values(headers, &state.config.auth.session_cookie_name);
    if tokens.is_empty() {
        return Err(AuthError::Unauthorized);
    }

    for token in &tokens {
        if let Ok(claims) = jwt::verify_session_jwt(token, jwt_secret) {
            let role = claims.user_role();
            return Ok(AuthIdentity {
                user_id: claims.sub,
                login: claims.login,
                role,
                provider: claims.provider,
                group_id: claims.group_id,
                team_ids: claims.team_ids,
            });
        }
    }

    Err(AuthError::SessionExpired)
}

// ============================================================================
// GET /api/v1/auth/verify — Traefik ForwardAuth endpoint
// ============================================================================

/// Verify authentication via session cookie or API key (Traefik ForwardAuth).
///
/// Returns 200/401 only — no response headers.
/// For preview URLs (hostname contains `--`), also checks task-based access.
#[utoipa::path(
    get,
    path = "/api/v1/auth/verify",
    tag = "auth",
    responses(
        (status = 200, description = "Authentication valid"),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody)
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

    let identity = authenticate_request(&state, &headers).await?;

    // For preview URLs, check task-based access control
    let forwarded_host = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok());
    if let Some(host) = forwarded_host {
        if let Some(task_id) = extract_task_id_from_hostname(host) {
            if let Some(ref tasks) = state.tasks {
                let scope_ctx = ScopeContext {
                    user_id: identity.user_id.clone(),
                    team_ids: identity.team_ids.clone(),
                    is_admin: identity.role == UserRole::Admin,
                };
                let task = tasks
                    .find_by_id(task_id, &scope_ctx)
                    .await
                    .map_err(|_| AuthError::Forbidden)?;
                if task.is_none() {
                    return Err(AuthError::Forbidden);
                }
            }
        }
    }

    Ok((StatusCode::OK, "OK").into_response())
}

/// Extract task ID from a preview hostname.
///
/// Format: `{task_id}--{port}.{proxy_domain}`
fn extract_task_id_from_hostname(host: &str) -> Option<&str> {
    let first_segment = host.split('.').next()?;
    if !first_segment.contains("--") {
        return None;
    }
    let task_id = first_segment.split("--").next()?;
    if task_id.is_empty() {
        None
    } else {
        Some(task_id)
    }
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
            providers: Vec::new(),
        }));
    }

    // Try API key first, fall back to session cookie
    let identity = authenticate_request(&state, &headers).await?;

    // Fetch fresh user data from DB (email/name/avatar may have changed)
    let user = state.users.get_user(&identity.user_id).await?;
    let Some(user) = user else {
        return Err(AuthError::SessionExpired);
    };

    let providers = check_provider_connections(&state, &identity.user_id).await;

    Ok(Json(SessionResponse {
        user: Some(UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: identity.provider,
            role: identity.role,
        }),
        expires_at: None,
        providers,
    }))
}

/// Inspect external provider tokens for the current user and report whether
/// each provider is currently usable. Returns a best-effort list: DB errors
/// are logged but surfaced as `connected: false` rather than failing `/me`.
///
/// Scoped to GitHub for now — extend with additional providers as they gain
/// refreshable credentials.
async fn check_provider_connections(state: &AuthState, user_id: &str) -> Vec<ProviderConnection> {
    let mut out = Vec::with_capacity(1);
    out.push(check_github_connection(state, user_id).await);
    out
}

async fn check_github_connection(state: &AuthState, user_id: &str) -> ProviderConnection {
    let disconnected = |login: Option<String>| ProviderConnection {
        provider: "github".to_string(),
        connected: false,
        login,
    };

    let token = match state
        .provider_tokens_dao
        .find_provider_token_full(user_id, "github")
        .await
    {
        Ok(row) => row,
        Err(e) => {
            warn!(
                error = %e,
                user_id = %user_id,
                "Failed to load GitHub provider token while building /me response"
            );
            return disconnected(None);
        }
    };

    let Some(token) = token else {
        return disconnected(None);
    };

    let login = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("login"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // When a GitHub App service is configured, verify the token by exercising
    // the refresh path. Any error (missing token row, refresh rejected, etc.)
    // is reported as disconnected so the frontend can prompt for reconnect.
    if let Some(ref app_svc) = state.github_app_service {
        match super::token_refresh::ensure_fresh_token(
            state.provider_tokens_dao.as_ref(),
            state.provider_tokens.as_ref(),
            app_svc,
            user_id,
        )
        .await
        {
            Ok(_) => ProviderConnection {
                provider: "github".to_string(),
                connected: true,
                login,
            },
            Err(e) => {
                warn!(
                    error = %e,
                    user_id = %user_id,
                    "GitHub token unavailable when reporting /me provider status"
                );
                disconnected(login)
            }
        }
    } else {
        // Without an App service we cannot refresh, so treat the stored token
        // as usable. Callers that actually hit GitHub will still surface 401s
        // through `RepositoryError::Forbidden` if the token has been revoked.
        ProviderConnection {
            provider: "github".to_string(),
            connected: true,
            login,
        }
    }
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
        .update_user_profile(&claims.sub, body.name.clone(), body.email.clone())
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
        providers: Vec::new(),
    }))
}

// ============================================================================
// GET /api/v1/auth/login — redirect to configured provider
// ============================================================================

async fn login_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    match state.config.auth.provider {
        None => Ok(Redirect::to(&state.config.auth.frontend_url).into_response()),
        Some(AuthProvider::GitHub) => Ok(Redirect::to("/api/v1/auth/github").into_response()),
        Some(AuthProvider::GitHubApp) => {
            Ok(Redirect::to("/api/v1/auth/github-app").into_response())
        }
        Some(AuthProvider::Auth0) => Ok(Redirect::to("/api/v1/auth/auth0").into_response()),
    }
}

// ============================================================================
// GET /api/v1/auth/cli/login — initiate OAuth for CLI login
// ============================================================================

/// CLI login: initiate OAuth flow with state encoding the CLI callback port.
///
/// The CSRF state is encoded as `{random}:cli:{port}` so that `handle_oauth_callback`
/// can detect the CLI flow and redirect to `http://localhost:{port}/callback`.
async fn cli_login_handler(
    State(state): State<AuthState>,
    axum::extract::Query(query): axum::extract::Query<CliLoginQuery>,
) -> Result<Response, AuthError> {
    let provider_id = state
        .config
        .auth
        .provider
        .ok_or_else(|| AuthError::Config("No OAuth provider configured".to_string()))?;

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

// ============================================================================
// OAuth init + callback helpers
// ============================================================================

fn generate_csrf_state() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

/// Initiate OAuth flow for a given provider.
async fn initiate_oauth(
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

    // Check required GitHub org membership
    if let Some(ref required_org) = state.config.auth.github_required_org {
        let is_member = provider
            .check_org_membership(&tokens.access_token, required_org)
            .await?;
        if !is_member {
            warn!(
                user_login = %user_info.login,
                required_org = %required_org,
                "User is not a member of required GitHub org"
            );
            let redirect = format!("{}?error=org_required", state.config.auth.frontend_url);
            return Ok((StatusCode::FOUND, [("Location", redirect.as_str())]).into_response());
        }
    }

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

    // Check if this is a CLI login flow (state contains ":cli:{port}")
    let cli_port = query_state.and_then(|qs| {
        let parts: Vec<&str> = qs.split(":cli:").collect();
        if parts.len() == 2 {
            parts[1].parse::<u16>().ok()
        } else {
            None
        }
    });

    if let Some(port) = cli_port {
        // CLI flow: generate API key and redirect to localhost callback
        let result = api_key::generate_key(
            state.api_keys.as_ref(),
            &user_info.id,
            "moat-cli",
            None,
            None,
            None,
        )
        .await?;

        let login_encoded = urlencoding::encode(&user_info.login);
        let key_encoded = urlencoding::encode(&result.key);
        let redirect_url =
            format!("http://127.0.0.1:{port}/callback?key={key_encoded}&login={login_encoded}");

        info!(user_login = %user_info.login, "CLI login successful, redirecting to localhost:{}", port);

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
    let session_cookie = cookie::build_session_cookie(
        &state.config,
        &jwt_token,
        state.config.auth.session_max_age_secs,
    );
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
    handle_oauth_callback(
        &state,
        AuthProvider::GitHub,
        &query.code,
        query.state.as_deref(),
        &headers,
    )
    .await
}

// ============================================================================
// GitHub App OAuth
// ============================================================================

async fn github_app_init_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::GitHubApp).await
}

async fn github_app_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    handle_oauth_callback(
        &state,
        AuthProvider::GitHubApp,
        &query.code,
        query.state.as_deref(),
        &headers,
    )
    .await
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
    handle_oauth_callback(
        &state,
        AuthProvider::Auth0,
        &query.code,
        query.state.as_deref(),
        &headers,
    )
    .await
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
            if let Some(logout_url) = provider.generate_logout_url(&state.config.auth.frontend_url)
            {
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

    let user = state
        .users
        .get_user(&payload.user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    let jwt_secret = require_jwt_secret(&state.config)?;

    let team_ids = if let Some(ref teams_repo) = state.teams {
        teams_repo
            .find_team_ids_for_user(&user.id)
            .await
            .unwrap_or_default()
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
        providers: Vec::new(),
    };

    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = session_cookie.parse() {
        resp_headers.insert(header::SET_COOKIE, v);
    }

    Ok((resp_headers, Json(body)).into_response())
}

// ============================================================================
// POST /api/v1/auth/token/session — exchange sandbox user token for session
// ============================================================================

/// Exchange a sandbox user auth token (JWT) for a session cookie.
///
/// Sandbox pods carry a `USER_AUTH_TOKEN` JWT signed with `TASK_TOKEN_SECRET`.
/// This endpoint verifies that token and creates a session, allowing the CLI
/// to call authenticated `/api/v1/*` endpoints.
#[utoipa::path(
    post,
    path = "/api/v1/auth/token/session",
    tag = "auth",
    responses(
        (status = 200, description = "Session created", body = TokenSessionResponse),
        (status = 401, description = "Invalid or missing token", body = ErrorBody)
    )
)]
async fn exchange_token_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let token = headers
        .get("X-User-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Unauthorized)?;

    let secret = state
        .config
        .sandbox
        .task_token_secret
        .as_deref()
        .ok_or_else(|| AuthError::Config("TASK_TOKEN_SECRET not configured".to_string()))?;

    let claims = crate::sandbox::jwt::verify_user_auth_token(token, secret)
        .map_err(|_| AuthError::Unauthorized)?;

    let user = state
        .users
        .get_user(&claims.user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    let jwt_secret = require_jwt_secret(&state.config)?;

    let team_ids = if let Some(ref teams_repo) = state.teams {
        teams_repo
            .find_team_ids_for_user(&user.id)
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };

    let jwt_token = session::create_session_from_user(
        state.sessions.as_ref(),
        &user,
        None, // sandbox tokens don't carry group_id
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

    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.auth.session_max_age_secs as i64);

    let body = TokenSessionResponse {
        token: jwt_token,
        user: UserInfo {
            id: user.id,
            login: user.login,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            provider: "token".to_string(),
            role: user.role,
        },
        expires_at: format_datetime(expires_at),
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

    let expires_at = body
        .expires_in_days
        .map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

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
// Provider ID management
// ============================================================================

/// Resolve a login to a user ID, returning 404 if not found.
async fn resolve_login_to_user_id(state: &AuthState, login: &str) -> Result<String, AuthError> {
    let user = state
        .users
        .get_user_by_login(login)
        .await?
        .ok_or_else(|| AuthError::NotFound(format!("User '{login}' not found")))?;
    Ok(user.id)
}

/// Set (upsert) a provider-specific user ID for a moatless user.
///
/// Only the user themselves or an admin may call this.
#[utoipa::path(
    put,
    path = "/api/v1/users/{login}/provider-ids/{provider}",
    tag = "users",
    params(
        ("login" = String, Path, description = "Moatless username"),
        ("provider" = String, Path, description = "Provider name, e.g. 'slack'")
    ),
    request_body = SetProviderIdRequest,
    responses(
        (status = 200, description = "Provider ID set", body = MessageResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
async fn set_user_provider_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((login, provider)): Path<(String, String)>,
    Json(body): Json<SetProviderIdRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    let user_id = resolve_login_to_user_id(&state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    state
        .users
        .upsert_provider_id(&user_id, &provider, &body.provider_user_id)
        .await?;

    Ok(Json(MessageResponse {
        message: format!("Provider ID for '{provider}' set successfully"),
    }))
}

/// Delete a provider ID mapping for a moatless user.
///
/// Only the user themselves or an admin may call this.
#[utoipa::path(
    delete,
    path = "/api/v1/users/{login}/provider-ids/{provider}",
    tag = "users",
    params(
        ("login" = String, Path, description = "Moatless username"),
        ("provider" = String, Path, description = "Provider name, e.g. 'slack'")
    ),
    responses(
        (status = 200, description = "Provider ID deleted", body = MessageResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody),
        (status = 404, description = "User or mapping not found", body = ErrorBody)
    )
)]
async fn delete_user_provider_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((login, provider)): Path<(String, String)>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    let user_id = resolve_login_to_user_id(&state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    let deleted = state.users.delete_provider_id(&user_id, &provider).await?;
    if !deleted {
        return Err(AuthError::NotFound(format!(
            "No '{provider}' provider ID found for user '{login}'"
        )));
    }

    Ok(Json(MessageResponse {
        message: format!("Provider ID for '{provider}' deleted"),
    }))
}

/// List all provider ID mappings for a moatless user.
///
/// Only the user themselves or an admin may call this.
#[utoipa::path(
    get,
    path = "/api/v1/users/{login}/provider-ids",
    tag = "users",
    params(
        ("login" = String, Path, description = "Moatless username")
    ),
    responses(
        (status = 200, description = "List of provider ID mappings", body = Vec<UserProviderIdItem>),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
async fn list_user_provider_ids_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
) -> Result<Json<Vec<UserProviderIdItem>>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    let user_id = resolve_login_to_user_id(&state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    let accounts = state.users.list_provider_ids(&user_id).await?;
    let items: Vec<UserProviderIdItem> = accounts
        .into_iter()
        .map(|a| UserProviderIdItem {
            provider: a.provider,
            provider_user_id: a.provider_user_id,
            created_at: format_datetime(a.created_at),
        })
        .collect();

    Ok(Json(items))
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
    let mut team_ids = teams
        .find_team_ids_for_user(user_id)
        .await
        .unwrap_or_default();

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

// ============================================================================
// Linear workspace OAuth install
// ============================================================================

/// Initiate the Linear workspace install flow.
///
/// Returns 302 to Linear's authorization URL with `actor=app` and the
/// workspace agent scopes. Sets an `oauth_state` cookie for CSRF.
/// Returns 503 when Linear OAuth credentials aren't configured.
async fn linear_install_handler(
    State(state): State<AuthState>,
) -> Result<Response, AuthError> {
    let Some(provider) = state.providers.linear.as_ref() else {
        return Err(AuthError::Config(
            "Linear OAuth not configured (set LINEAR_OAUTH_CLIENT_ID, \
             LINEAR_OAUTH_CLIENT_SECRET, LINEAR_OAUTH_CALLBACK_URL)"
                .into(),
        ));
    };

    let csrf_state = generate_csrf_state();
    let auth_url = provider.generate_auth_url(&csrf_state).ok_or_else(|| {
        AuthError::Config("Failed to generate Linear OAuth URL".to_string())
    })?;

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
/// 4. Upsert `integration_connections(provider="linear", external_account_id=org_id)`.
/// 5. Upsert `provider_tokens(team_id=org_id, provider="linear")`.
async fn linear_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<super::schemas::OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    let provider = state.providers.linear.as_ref().ok_or_else(|| {
        AuthError::Config("Linear OAuth not configured".into())
    })?;

    let stored_state = cookie::get_cookie_value(&headers, "oauth_state");
    match (stored_state.as_deref(), query.state.as_deref()) {
        (Some(stored), Some(qs)) if stored == qs => { /* CSRF check passed */ }
        _ => {
            warn!("Linear OAuth state mismatch or missing — possible CSRF attack");
            return Err(AuthError::InvalidState);
        }
    }

    let tokens = provider.exchange_code_for_tokens(&query.code).await?;
    let identity = provider
        .fetch_install_identity(&tokens.access_token)
        .await?;

    // Upsert the workspace-level integration_connections row.
    use crate::adapters::dao::PgAdapterConnectionRepository;
    use crate::adapters::models::AdapterConnection;
    let conn_repo = PgAdapterConnectionRepository(state.db.clone());

    let now = chrono::Utc::now();
    let transport_config = serde_json::json!({
        "source": "oauth",
        "agent_viewer_id": identity.agent_viewer_id,
        "agent_viewer_name": identity.agent_viewer_name,
        "organization_name": identity.organization_name,
    });

    let existing = conn_repo
        .find_by_external_account("linear", &identity.organization_id)
        .await
        .map_err(AuthError::Database)?;

    let connection_row = AdapterConnection {
        id: existing
            .as_ref()
            .map_or_else(|| format!("pc_{}", uuid::Uuid::new_v4()), |c| c.id.clone()),
        provider: "linear".to_string(),
        connection_kind: "workspace".to_string(),
        scope_value: identity.organization_id.clone(),
        credential_ref: None,
        webhook_secret: existing.as_ref().and_then(|c| c.webhook_secret.clone()),
        transport_config,
        created_by: existing
            .as_ref()
            .map_or_else(|| "system".to_string(), |c| c.created_by.clone()),
        scope: existing
            .as_ref()
            .map_or(crate::scope::Scope::Global, |c| c.scope),
        team_id: existing.as_ref().and_then(|c| c.team_id.clone()),
        owner_user_id: existing.as_ref().and_then(|c| c.owner_user_id.clone()),
        created_at: existing.as_ref().map_or(now, |c| c.created_at),
        updated_at: now,
    };
    conn_repo
        .upsert(&connection_row)
        .await
        .map_err(AuthError::Database)?;

    // Upsert the workspace token row.
    let expires_at = tokens
        .expires_in
        .map(|secs| now + chrono::Duration::seconds(secs as i64));
    let metadata = serde_json::json!({
        "agent_viewer_id": identity.agent_viewer_id,
        "agent_viewer_name": identity.agent_viewer_name,
        "agent_viewer_email": identity.agent_viewer_email,
        "scope": tokens.scope,
    });
    state
        .provider_tokens_dao
        .upsert_team_token(
            &identity.organization_id,
            "linear",
            &tokens.access_token,
            tokens.refresh_token.as_deref(),
            Some(tokens.token_type.as_str()),
            Some(&metadata),
        )
        .await
        .map_err(AuthError::Database)?;

    if let Some(expires) = expires_at {
        sqlx::query(
            "UPDATE provider_tokens SET expires_at = $1 \
             WHERE team_id = $2 AND provider = 'linear' AND user_id IS NULL",
        )
        .bind(expires)
        .bind(&identity.organization_id)
        .execute(&state.db)
        .await
        .map_err(AuthError::Database)?;
    }

    info!(
        org_id = %identity.organization_id,
        agent_id = %identity.agent_viewer_id,
        "Linear workspace installed"
    );

    let clear_state_cookie = cookie::build_clear_oauth_state_cookie(&state.config);
    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = clear_state_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }
    let redirect = format!(
        "{}?linear_installed={}",
        state.config.auth.frontend_url,
        urlencoding::encode(&identity.organization_id)
    );
    Ok((
        StatusCode::FOUND,
        resp_headers,
        [("Location", redirect.as_str())],
    )
        .into_response())
}
