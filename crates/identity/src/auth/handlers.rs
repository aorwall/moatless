//! Authentication HTTP adaptation.

use super::accounts::require_local_auth;
use super::state::AuthState;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use rand::RngExt;
use tracing::{info, warn};
use urlencoding;

use moatless_api_schemas::error::ErrorBody;
use moatless_types::access_control::ScopeContext;
use moatless_types::auth::AuthProvider;

use super::api_key;
use super::cookie;
use super::error::AuthError;
use super::forgejo_connect;
use super::github_connect;
use moatless_api_schemas::user::{SetProviderIdRequest, UserProviderIdItem};
use moatless_types::auth::AuthIdentity;
use moatless_types::user::UserRole;

use super::schemas::{
    AdminResetPasswordRequest, AdminUpdateUserRequest, ApiKeyCreateResponse, ApiKeyListItem,
    AuthMode, AuthModeResponse, ChangePasswordRequest, CliLoginQuery, CreateApiKeyRequest,
    CreatePairingCodeRequest, CreateUserRequest, CreateUserResponse, ListUsersQuery,
    MessageResponse,
    OAuthCallbackQuery, PairingCodeResponse, PasswordLoginRequest, SessionResponse,
    TokenSessionResponse, UpdateUserRequest, UserInfo, UserListItem, UserListResponse,
    format_datetime, format_datetime_opt,
};
use super::sessions as session;
use super::sessions::IssuedAuthSession;

// ============================================================================
// Local email/password authentication
// ============================================================================

fn issued_session_headers(state: &AuthState, issued: &IssuedAuthSession) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let session_cookie = cookie::build_session_cookie(
        &state.config,
        &issued.token,
        state.config.auth.session_absolute_age_secs,
    );
    let csrf_cookie = cookie::build_csrf_cookie(
        &state.config,
        &issued.csrf_token,
        state.config.auth.session_absolute_age_secs,
    );
    if let Ok(value) = session_cookie.parse() {
        headers.append(header::SET_COOKIE, value);
    }
    if let Ok(value) = csrf_cookie.parse() {
        headers.append(header::SET_COOKIE, value);
    }
    headers
}

async fn revoke_present_session(state: &AuthState, headers: &HeaderMap) -> Result<(), AuthError> {
    if let Some(token) = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name) {
        session::revoke_session(state.sessions.as_ref(), &token).await?;
    }
    Ok(())
}

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
pub(super) async fn auth_mode_handler(State(state): State<AuthState>) -> Json<AuthModeResponse> {
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

/// Log in with an email and password, returning an auth-session cookie.
#[utoipa::path(
    post,
    path = "/api/v1/auth/password/login",
    tag = "auth",
    request_body = PasswordLoginRequest,
    responses(
        (status = 200, description = "Logged in", body = SessionResponse),
        (status = 401, description = "Invalid credentials", body = ErrorBody),
        (status = 404, description = "Password auth not enabled", body = ErrorBody),
        (status = 429, description = "Too many failed attempts", body = ErrorBody),
    )
)]
pub(super) async fn password_login_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AuthError> {
    let present_token = cookie::get_cookie_value(&headers, &state.config.auth.session_cookie_name);
    let (issued, response) = super::password_operations::login(
        &state,
        req,
        request_source(&headers),
        present_token.as_deref(),
    )
    .await?;
    Ok((issued_session_headers(&state, &issued), Json(response)).into_response())
}

fn request_source(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Change your own password.
#[utoipa::path(
    put,
    path = "/api/v1/auth/password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Password changed", body = MessageResponse),
        (status = 400, description = "Password too short", body = ErrorBody),
        (status = 401, description = "Not authenticated or wrong current password", body = ErrorBody),
        (status = 404, description = "Password auth not enabled", body = ErrorBody),
    )
)]
pub(super) async fn change_password_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    require_local_auth(&state)?;
    // Password changes require a session cookie, never an API key.
    let claims = authenticate_from_cookie(&state, &headers).await?;
    super::password_operations::change_password(&state, claims.sub, req).await?;
    Ok(Json(MessageResponse {
        message: "Password changed successfully".to_string(),
    }))
}

/// Reset another user's password. Admin only — no current password required,
/// so a forgotten password has a recovery path that isn't env surgery.
#[utoipa::path(
    put,
    path = "/api/v1/users/{login}/password",
    tag = "users",
    params(("login" = String, Path, description = "Target user login")),
    request_body = AdminResetPasswordRequest,
    responses(
        (status = 200, description = "Password reset", body = MessageResponse),
        (status = 400, description = "Password too short", body = ErrorBody),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Admin only", body = ErrorBody),
        (status = 404, description = "User has no local credential", body = ErrorBody),
    )
)]
pub(super) async fn admin_reset_password_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
    Json(req): Json<AdminResetPasswordRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    require_local_auth(&state)?;
    let identity = authenticate_request(&state, &headers).await?;
    super::password_operations::reset_password(&state, &identity, &login, req).await?;
    Ok(Json(MessageResponse {
        message: "Password reset successfully".to_string(),
    }))
}

// ============================================================================
// Helper: resolve JWT secret from config
// ============================================================================

// ============================================================================
// Helper: authenticate from an opaque auth-session cookie
// ============================================================================

struct CookieSession {
    sub: uuid::Uuid,
    role: UserRole,
    provider: String,
    group_id: Option<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl CookieSession {
    fn user_role(&self) -> UserRole {
        self.role
    }
}

async fn authenticate_from_cookie(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<CookieSession, AuthError> {
    let token = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name)
        .ok_or(AuthError::Unauthorized)?;
    let resolved = session::validate_session(
        state.sessions.as_ref(),
        &token,
        state.config.auth.session_max_age_secs,
    )
    .await?
    .ok_or(AuthError::SessionExpired)?;
    Ok(CookieSession {
        sub: resolved.user_id,
        role: resolved.role,
        provider: resolved.authenticated_via,
        group_id: resolved.group_id,
        expires_at: resolved.absolute_expires_at,
    })
}

// ============================================================================
// Shared auth helper
// ============================================================================

/// Authenticate from session cookie or API key.
///
/// Used by both `verify_handler` (`ForwardAuth`) and internal callers.
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
                let user_id = payload
                    .user_id
                    .parse::<uuid::Uuid>()
                    .map_err(|_| AuthError::InvalidApiKey)?;
                return Ok(AuthIdentity {
                    user_id,
                    login: payload.user_login,
                    role: payload.role,
                    provider: payload.provider,
                    group_id: payload.group_id,
                });
            }
            return Err(AuthError::InvalidApiKey);
        }
    }

    // Try opaque auth-session cookies.
    let tokens = cookie::get_all_cookie_values(headers, &state.config.auth.session_cookie_name);
    if tokens.is_empty() {
        return Err(AuthError::Unauthorized);
    }

    for token in &tokens {
        if let Some(auth_session) = session::validate_session(
            state.sessions.as_ref(),
            token,
            state.config.auth.session_max_age_secs,
        )
        .await?
        {
            return Ok(AuthIdentity {
                user_id: auth_session.user_id,
                login: auth_session.login,
                role: auth_session.role,
                provider: auth_session.authenticated_via,
                group_id: auth_session.group_id,
            });
        }
    }

    Err(AuthError::SessionExpired)
}

// ============================================================================
// GET /api/v1/auth/verify — Traefik ForwardAuth endpoint
// ============================================================================

/// Verify authentication via session cookie or API key (Traefik `ForwardAuth`).
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
pub(super) async fn verify_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;

    // For preview URLs, check task-based access control.
    let scope_ctx = ScopeContext {
        user_id: identity.user_id.to_string(),
        is_admin: identity.role == UserRole::Admin,
    };
    let forwarded_host = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok());
    enforce_preview_access(state.task_access.as_ref(), &scope_ctx, forwarded_host).await?;

    Ok((StatusCode::OK, "OK").into_response())
}

/// The preview-URL session gate: when `forwarded_host` names a preview host,
/// `Forbidden` unless `scope_ctx` may reach its task. A non-preview host (or none)
/// passes without consulting the port. Split out so the gate is testable without
/// the authentication machinery in front of it.
async fn enforce_preview_access(
    task_access: &dyn moatless_core::http_auth::TaskAccess,
    scope_ctx: &ScopeContext,
    forwarded_host: Option<&str>,
) -> Result<(), AuthError> {
    let Some(host) = forwarded_host else {
        return Ok(());
    };
    let Some(task_id) = extract_task_id_from_hostname(host) else {
        return Ok(());
    };
    if task_access.may_access_task(task_id, scope_ctx).await {
        Ok(())
    } else {
        Err(AuthError::Forbidden)
    }
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
pub(super) async fn me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthError> {
    // Try API key first, fall back to session cookie
    let identity = authenticate_request(&state, &headers).await?;

    super::account_status::current_user(&state, identity)
        .await
        .map(Json)
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
pub(super) async fn update_me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<SessionResponse>, AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;

    let role = claims.user_role();

    // Update the users table
    let user = state
        .users
        .update_user_profile(&claims.sub, body.name.clone(), body.email.clone())
        .await?
        .ok_or_else(|| AuthError::NotFound("User not found".to_string()))?;

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
        expires_at: Some(format_datetime(claims.expires_at)),
        providers: Vec::new(),
        setup: None,
    }))
}

// ============================================================================
// GET /api/v1/auth/login — redirect to configured provider
// ============================================================================

pub(super) async fn login_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
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

// ============================================================================
// GET /api/v1/auth/cli/login — initiate OAuth for CLI login
// ============================================================================

/// CLI login: initiate OAuth flow with state encoding the CLI callback port.
///
/// The CSRF state is encoded as `{random}:cli:{port}` so that `handle_oauth_callback`
/// can detect the CLI flow and redirect to `http://localhost:{port}/callback`.
pub(super) async fn cli_login_handler(
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
/// `code` is absent when the person declined on the provider's consent screen,
/// which is a normal outcome rather than a malformed request.
async fn handle_oauth_callback(
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
    let outcome = super::oauth::login::complete_login(
        state,
        provider.as_ref(),
        provider_id,
        code,
        present_token.as_deref(),
        cli_port.is_some(),
    )
    .await?;
    let (issued, login, cli_key) = match outcome {
        super::oauth::login::LoginOutcome::OrganizationRequired => {
            let redirect = format!("{}?error=org_required", state.config.auth.frontend_url);
            return Ok((StatusCode::FOUND, [("Location", redirect.as_str())]).into_response());
        }
        super::oauth::login::LoginOutcome::Completed {
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

// ============================================================================
// GitHub App OAuth
// ============================================================================

pub(super) async fn github_app_init_handler(
    State(state): State<AuthState>,
) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::GitHubApp).await
}

#[derive(serde::Deserialize)]
pub(super) struct GitHubConnectQuery {
    /// Where to send the browser once the App is authorized. Vetted against the
    /// trusted origins — see [`super::connect_return::approved_return_url`].
    return_to: Option<String>,
}

/// Authorize the GitHub App for the caller's existing session.
///
/// Requires a session because the credential is written against it: this
/// establishes GitHub *access*, and leaves how the caller signed in alone.
pub(super) async fn github_app_connect_handler(
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

pub(super) async fn github_app_callback_handler(
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

// ============================================================================
// Forgejo OAuth
// ============================================================================

#[derive(serde::Deserialize)]
pub(super) struct ForgejoConnectQuery {
    /// Which instance to connect. Forgejo is self-hostable, so it selects the
    /// registered OAuth2 application, the credential row and the secret.
    host: String,
    /// Where to send the browser once the instance is authorized. Vetted
    /// against the trusted origins — see
    /// [`super::connect_return::approved_return_url`].
    return_to: Option<String>,
}

/// Authorize a Forgejo instance for the caller's existing session.
///
/// Requires a session because the credential is written against it: this
/// establishes Forgejo *access*, and leaves how the caller signed in alone.
pub(super) async fn forgejo_connect_handler(
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

pub(super) async fn forgejo_callback_handler(
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

// ============================================================================
// Auth0 OAuth
// ============================================================================

pub(super) async fn auth0_init_handler(
    State(state): State<AuthState>,
) -> Result<Response, AuthError> {
    initiate_oauth(&state, AuthProvider::Auth0).await
}

pub(super) async fn auth0_callback_handler(
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

// ============================================================================
// OpenID Connect
// ============================================================================

#[derive(serde::Deserialize)]
pub(super) struct OidcStartQuery {
    return_to: Option<String>,
    port: Option<i32>,
}

pub(super) async fn oidc_start_handler(
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

pub(super) async fn oidc_callback_handler(
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

// ============================================================================
// Logout
// ============================================================================

async fn do_logout(state: &AuthState, headers: &HeaderMap) -> Result<Response, AuthError> {
    let mut session_provider: Option<AuthProvider> = None;
    if let Some(token) = cookie::get_cookie_value(headers, &state.config.auth.session_cookie_name) {
        if let Some(auth_session) = session::validate_session(
            state.sessions.as_ref(),
            &token,
            state.config.auth.session_max_age_secs,
        )
        .await?
        {
            session_provider = auth_session.authenticated_via.parse().ok();
        }
        session::revoke_session(state.sessions.as_ref(), &token).await?;
    }

    let clear_cookie = cookie::build_clear_session_cookie(&state.config);
    let mut resp_headers = HeaderMap::new();
    if let Ok(v) = clear_cookie.parse() {
        resp_headers.append(header::SET_COOKIE, v);
    }
    if let Ok(v) = cookie::build_clear_csrf_cookie(&state.config).parse() {
        resp_headers.append(header::SET_COOKIE, v);
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
pub(super) async fn logout_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    do_logout(&state, &headers).await
}

pub(super) async fn logout_get_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    do_logout(&state, &headers).await
}

/// Revoke every auth session belonging to the current user.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    tag = "auth",
    responses((status = 200, description = "All sessions revoked", body = MessageResponse))
)]
pub(super) async fn logout_all_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let current = authenticate_from_cookie(&state, &headers).await?;
    state
        .sessions
        .delete_sessions_for_user(&current.sub.to_string())
        .await?;
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
pub(super) async fn exchange_api_key_handler(
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

    let user_id = payload
        .user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AuthError::Unauthorized)?;
    let user = state
        .users
        .get_user(&user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    revoke_present_session(&state, &headers).await?;
    let issued = session::create_session_from_user(
        state.sessions.as_ref(),
        &user,
        payload.group_id.as_deref(),
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
    )
    .await?;

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
        setup: None,
    };

    let resp_headers = issued_session_headers(&state, &issued);

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
pub(super) async fn exchange_token_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Response, AuthError> {
    let token = headers
        .get("X-User-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthError::Unauthorized)?;

    let verified = state
        .token_verifier
        .verify_user_token(token)
        .map_err(|e| match e {
            moatless_core::http_auth::TokenVerifyError::NotConfigured => {
                AuthError::Config("TASK_TOKEN_SECRET not configured".to_string())
            }
            moatless_core::http_auth::TokenVerifyError::Invalid(_) => AuthError::Unauthorized,
        })?;

    let user_id = verified
        .user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AuthError::Unauthorized)?;
    let user = state
        .users
        .get_user(&user_id)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    revoke_present_session(&state, &headers).await?;
    let issued = session::create_session_from_user(
        state.sessions.as_ref(),
        &user,
        None, // sandbox tokens don't carry group_id
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
    )
    .await?;

    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.auth.session_absolute_age_secs as i64);

    let body = TokenSessionResponse {
        token: issued.token.clone(),
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

    let resp_headers = issued_session_headers(&state, &issued);

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
pub(super) async fn create_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreateResponse>), AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;

    let expires_at = body
        .expires_in_days
        .map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

    let result = api_key::generate_key(
        state.api_keys.as_ref(),
        &claims.sub.to_string(),
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

/// Mint a one-time code that pairs a client on another device to this account.
///
/// The code is the bootstrap half of pairing: the client exchanges it once, at
/// `POST /oauth/token`, for the API key it then keeps. It is readable in this
/// response and nowhere afterwards.
#[utoipa::path(
    post,
    path = "/api/v1/auth/pairing-codes",
    tag = "auth",
    request_body = CreatePairingCodeRequest,
    responses(
        (status = 201, description = "Pairing code created", body = PairingCodeResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
pub(super) async fn create_pairing_code_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<CreatePairingCodeRequest>,
) -> Result<(StatusCode, Json<PairingCodeResponse>), AuthError> {
    // Cookie only, deliberately. A pairing code is a credential that mints
    // another credential, so the one thing that may ask for it is a person at a
    // browser they logged into — not a key, and least of all a key some other
    // device was paired with.
    let claims = authenticate_from_cookie(&state, &headers).await?;

    let issued = state
        .pairing
        .issue_code(
            &claims.sub.to_string(),
            claims.group_id.as_deref(),
            body.label.as_deref(),
        )
        .await
        .map_err(|error| AuthError::Internal(error.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(PairingCodeResponse {
            id: issued.id,
            code: issued.code,
            label: issued.label,
            expires_at: format_datetime(issued.expires_at),
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
pub(super) async fn list_api_keys_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeyListItem>>, AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;
    let keys = api_key::list_keys(state.api_keys.as_ref(), &claims.sub.to_string()).await?;

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
pub(super) async fn revoke_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<MessageResponse>, AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;
    let result =
        api_key::revoke_key(state.api_keys.as_ref(), &key_id, &claims.sub.to_string()).await?;

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

/// Create a user. Admin only.
///
/// With `isBot: true`, resolves the App's `{slug}[bot]` identity via
/// `GitHubAppRegistry`, then inserts both the `users` row and a matching
/// `provider_tokens` row with `auth_method = 'github_app_installation'` so
/// credential resolution transparently mints installation tokens for tasks
/// that `run_as_user_id = <bot id>`.
///
/// With `isBot: false`, creates a human user with a local password. Available
/// only under local password auth — OAuth deployments create users on callback.
#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "users",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = CreateUserResponse),
        (status = 400, description = "Unknown app key, or missing login/password", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Admin only", body = ErrorBody),
        (status = 409, description = "Login already taken", body = ErrorBody),
        (status = 500, description = "Internal error", body = ErrorBody),
    )
)]
pub(super) async fn create_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateUserResponse>), AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    let created = super::accounts::create_user(&state, &identity, req).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// List the user directory.
#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    params(("includeDisabled" = Option<bool>, Query, description = "Include disabled accounts")),
    responses(
        (status = 200, description = "List of users", body = UserListResponse),
        (status = 500, description = "Internal error", body = ErrorBody)
    )
)]
pub(super) async fn list_users_handler(
    State(state): State<AuthState>,
    axum::extract::Query(query): axum::extract::Query<ListUsersQuery>,
) -> Result<Json<UserListResponse>, AuthError> {
    let users = state.users.list_users(query.include_disabled).await?;

    let items: Vec<UserListItem> = users
        .iter()
        .map(|u| UserListItem {
            id: u.id,
            login: u.login.clone(),
            email: u.email.clone(),
            name: u.name.clone(),
            avatar_url: u.avatar_url.clone(),
            provider: u.provider.clone(),
            role: u.role,
            created_at: format_datetime(u.created_at),
            last_login_at: format_datetime(u.last_login_at),
            is_bot: u.is_bot,
            status: u.status,
        })
        .collect();

    Ok(Json(UserListResponse { users: items }))
}

/// Update a user's profile (name and/or email).
///
/// A user may update their own profile; updating another user requires the
/// admin role. Backs the `moat user set-email <login>` CLI command so emails
/// can be corrected without direct DB access. Only the fields present in the
/// body are changed.
#[utoipa::path(
    patch,
    path = "/api/v1/users/{login}",
    tag = "users",
    params(("login" = String, Path, description = "Moatless username")),
    request_body = AdminUpdateUserRequest,
    responses(
        (status = 200, description = "User updated", body = UserInfo),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
pub(super) async fn update_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
    Json(body): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserInfo>, AuthError> {
    super::accounts::validate_profile_update(&body)?;
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        super::accounts::update_user(&state, &identity, login, body).await?,
    ))
}

/// Update the user an internal id names. Admin only.
///
/// A login is not unique — the same handle exists once per provider — and
/// `PATCH /api/v1/users/{login}` resolves one to its oldest row. Reach any
/// newer row of a shared login here, keyed on the `id` that `GET /api/v1/users`
/// already reports. The body is the one `PATCH /api/v1/users/{login}` takes.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/users/{user_id}",
    tag = "users",
    params(("user_id" = uuid::Uuid, Path, description = "Target user id")),
    request_body = AdminUpdateUserRequest,
    responses(
        (status = 200, description = "User updated", body = UserInfo),
        (status = 400, description = "An admin cannot disable their own account", body = ErrorBody),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Admin only", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
pub(super) async fn admin_update_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<uuid::Uuid>,
    Json(body): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserInfo>, AuthError> {
    super::accounts::validate_profile_update(&body)?;
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        super::accounts::update_user_by_id(&state, &identity, user_id, body).await?,
    ))
}

// ============================================================================
// Provider ID management
// ============================================================================

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
pub(super) async fn set_user_provider_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((login, provider)): Path<(String, String)>,
    Json(body): Json<SetProviderIdRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        super::accounts::set_user_provider_id(&state, &identity, login, provider, body).await?,
    ))
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
pub(super) async fn delete_user_provider_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((login, provider)): Path<(String, String)>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        super::accounts::delete_user_provider_id(&state, &identity, login, provider).await?,
    ))
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
pub(super) async fn list_user_provider_ids_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
) -> Result<Json<Vec<UserProviderIdItem>>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        super::accounts::list_user_provider_ids(&state, &identity, login).await?,
    ))
}

// ============================================================================
// Linear workspace OAuth install
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A `TaskAccess` that answers with a fixed verdict and counts its calls,
    /// so a test can assert both the returned decision and whether the gate
    /// consulted the port at all.
    struct FakeTaskAccess {
        allow: bool,
        calls: AtomicUsize,
    }

    impl FakeTaskAccess {
        fn new(allow: bool) -> Self {
            Self {
                allow,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl moatless_core::http_auth::TaskAccess for FakeTaskAccess {
        async fn may_access_task(&self, _task_id: &str, _ctx: &ScopeContext) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.allow
        }
    }

    fn scope_ctx() -> ScopeContext {
        ScopeContext {
            user_id: "user-1".to_string(),
            is_admin: false,
        }
    }

    #[tokio::test]
    async fn preview_gate_denies_task_the_scope_cannot_see() {
        let access = FakeTaskAccess::new(false);
        let result = enforce_preview_access(
            &access,
            &scope_ctx(),
            Some("task-1--abc.preview.example.com"),
        )
        .await;
        assert!(matches!(result, Err(AuthError::Forbidden)));
        assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn preview_gate_allows_task_the_scope_can_see() {
        let access = FakeTaskAccess::new(true);
        let result = enforce_preview_access(
            &access,
            &scope_ctx(),
            Some("task-1--abc.preview.example.com"),
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn non_preview_host_passes_without_consulting_the_port() {
        // A host with no `--` segment is not a preview URL; the gate must not
        // deny it and must not touch the port.
        let access = FakeTaskAccess::new(false);
        let result = enforce_preview_access(&access, &scope_ctx(), Some("app.example.com")).await;
        assert!(result.is_ok());
        assert_eq!(access.calls.load(Ordering::SeqCst), 0);
    }
}
