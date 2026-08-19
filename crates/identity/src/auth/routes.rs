//! Auth API route handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use rand::RngExt;
use tracing::{info, warn};
use urlencoding;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::secret::service::SecretService;
use crate::setup::{SetupContext, evaluate_setup};
use moatless_adapter_model::dao::AdapterConnectionDao;
use moatless_types::access_control::ScopeContext;
use moatless_core::config::Config;
use moatless_types::auth::AuthProvider;
use moatless_core::db::DbPool;
use moatless_api_schemas::error::ErrorBody;

use super::api_key;
use super::api_keys::ApiKeyDao;
use super::cache::AuthCache;
use super::cookie;
use super::error::AuthError;
use super::github::registry::GitHubAppRegistry;
use super::github::service::GitHubAppService;
use super::identity::AuthIdentity;
use moatless_types::user::UserRole;
use super::oauth::provider::OAuthProvider;
use super::oidc::OidcService;
use super::provider_token_dao::ProviderTokenDao;
use super::provider_tokens::ProviderTokenAuthDao;
use moatless_api_schemas::user::{SetProviderIdRequest, UserProviderIdItem};

use super::pairing::DevicePairing;
use super::password::dao::UserCredentialDao;
use super::password::hash;
use super::password::throttle::LoginThrottle;
use super::schemas::{AdminResetPasswordRequest, AdminUpdateUserRequest, ApiKeyCreateResponse, ApiKeyListItem, AuthMode, AuthModeResponse, ChangePasswordRequest, CliLoginQuery, CreateApiKeyRequest, CreatePairingCodeRequest, CreateUserRequest, CreateUserResponse, MessageResponse, OAuthCallbackQuery, PairingCodeResponse, PasswordLoginRequest, ProviderConnection, SessionResponse, TokenSessionResponse, UpdateUserRequest, UserInfo, UserListItem, UserListResponse, format_datetime, format_datetime_opt};
use super::session;
use super::session::IssuedAuthSession;
use super::sessions::SessionDao;
use super::users::UserDao;

/// Type-safe container for configured OAuth providers.
#[derive(Clone, Default)]
pub(super) struct OAuthProviders {
    pub github_app: Option<Arc<dyn OAuthProvider>>,
    pub auth0: Option<Arc<dyn OAuthProvider>>,
    /// Linear workspace install. Not used for user login — issued tokens are
    /// stored globally and consumed by the Linear Agents API client.
    pub linear: Option<Arc<super::oauth::linear::LinearOAuthProvider>>,
}

impl OAuthProviders {
    /// Build providers from application config.
    ///
    /// Each provider is only created when its required env vars are set.
    pub(super) fn from_config(config: &Config) -> Self {
        let github_app =
            super::oauth::github_app::GitHubAppOAuthProvider::from_config(config).map(|p| {
                tracing::info!("GitHub App OAuth provider configured");
                Arc::new(p) as Arc<dyn OAuthProvider>
            });
        if config.auth.provider == AuthProvider::GitHubApp && github_app.is_none() {
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
            github_app,
            auth0,
            linear,
        }
    }

    /// Look up a provider by AuthProvider enum.
    pub fn get(&self, provider: AuthProvider) -> Option<&Arc<dyn OAuthProvider>> {
        match provider {
            // Local password auth has no OAuth provider.
            AuthProvider::Local => None,
            AuthProvider::GitHubApp => self.github_app.as_ref(),
            AuthProvider::Auth0 => self.auth0.as_ref(),
            // Standards-compliant OIDC has its own typed client and does not
            // go through the provider-specific OAuth abstraction.
            AuthProvider::Oidc => None,
        }
    }
}

/// Shared state for auth routes.
#[derive(Clone)]
pub(super) struct AuthState {
    pub(super) db: DbPool,
    pub(super) config: Arc<Config>,
    pub(super) providers: Arc<OAuthProviders>,
    pub(super) oidc: Option<Arc<OidcService>>,
    pub(super) cache: AuthCache,
    pub(super) sessions: Arc<dyn SessionDao>,
    pub(super) api_keys: Arc<dyn ApiKeyDao>,
    /// Mints the codes that pair a client on another device to an account. The
    /// exchange half runs in the T3 compatibility module, against this same
    /// service.
    pub(super) pairing: Arc<DevicePairing>,
    pub(super) users: Arc<dyn UserDao>,
    /// Local password credentials (`user_credentials`).
    pub(super) credentials: Arc<dyn UserCredentialDao>,
    /// Shared per-email and per-source failed-login counters.
    pub(super) login_throttle: LoginThrottle,
    pub(super) provider_tokens: Arc<dyn ProviderTokenAuthDao>,
    /// DAO for credential lookups (full-row reads, refresh persistence).
    /// Separate from `provider_tokens` (which handles upsert/metadata paths).
    pub(super) provider_tokens_dao: Arc<dyn ProviderTokenDao>,
    /// Optional GitHub App service used to refresh expiring user tokens when
    /// reporting provider connection status on `/me`. Convenience handle to
    /// `github_apps.default_service()`.
    pub(super) github_app_service: Option<Arc<GitHubAppService>>,
    /// The one way to a git-host token. `/me` uses it to check whether the
    /// caller's own GitHub credential still resolves.
    pub(super) git_host_tokens: Arc<crate::git_host_tokens::GitHostTokens>,
    /// Registry of all configured GitHub Apps.
    pub(super) github_apps: Arc<GitHubAppRegistry>,
    /// Encrypted secret store — holds the OAuth token stashed behind a PAT
    /// override, and the validated PAT itself.
    pub(super) secrets: Arc<SecretService>,
    /// Preview-URL access for the session gate — the one `may_access_task` bool
    /// the ForwardAuth gate needs (E-(b)), behind a port so this crate holds no
    /// `task` import.
    pub(super) task_access: Arc<dyn crate::extractors::TaskAccess>,
    /// Verifies the sandbox-issued `X-User-Token` on `exchange_token`, behind a
    /// port so this crate holds neither the token secret nor a `sandbox` import.
    pub(super) token_verifier: Arc<dyn crate::extractors::UserTokenVerifier>,
    /// Adapter connections (`adapter_connections`). The Linear OAuth
    /// callback mirrors the authorized workspace into this table so inbound
    /// Linear webhooks resolve a connection.
    pub(super) adapter_connections: Arc<dyn AdapterConnectionDao>,
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
        auth_mode_handler,
        password_login_handler,
        change_password_handler,
        admin_reset_password_handler,
        me_handler,
        update_me_handler,
        logout_handler,
        logout_all_handler,
        exchange_api_key_handler,
        exchange_token_handler,
        create_api_key_handler,
        list_api_keys_handler,
        revoke_api_key_handler,
        create_pairing_code_handler,
        list_users_handler,
        create_user_handler,
        update_user_handler,
        set_user_provider_id_handler,
        delete_user_provider_id_handler,
        list_user_provider_ids_handler,
    )
)]
pub struct AuthApiDoc;

/// Create the auth router with all endpoints.
pub(super) fn auth_router(state: AuthState) -> (axum::Router, utoipa::openapi::OpenApi) {
    // Build OpenAPI-documented routes
    let (openapi_router, api) = documented_auth_routes()
        .with_state(state.clone())
        .split_for_parts();

    // Build a separate plain Router for OAuth redirect routes (not in OpenAPI — browser-only flows)
    let oauth_router = axum::Router::new()
        .route("/api/v1/auth/login", get(login_handler))
        .route("/api/v1/auth/cli/login", get(cli_login_handler))
        .route("/api/v1/auth/github-app", get(github_app_init_handler))
        .route(
            "/api/v1/auth/github-app/callback",
            get(github_app_callback_handler),
        )
        .route("/api/v1/auth/auth0", get(auth0_init_handler))
        .route("/api/v1/auth/auth0/callback", get(auth0_callback_handler))
        .route(
            "/api/v1/auth/oidc/{provider}/start",
            get(oidc_start_handler),
        )
        .route(
            "/api/v1/auth/oidc/{provider}/callback",
            get(oidc_callback_handler),
        )
        .route("/api/v1/auth/linear/install", get(linear_install_handler))
        .route("/api/v1/auth/linear/callback", get(linear_callback_handler))
        .route("/api/v1/auth/logout", get(logout_get_handler))
        .with_state(state);

    // Merge both into a single Router
    let router: axum::Router = Into::<axum::Router>::into(openapi_router).merge(oauth_router);

    (router, api)
}

fn documented_auth_routes() -> OpenApiRouter<AuthState> {
    OpenApiRouter::with_openapi(AuthApiDoc::openapi())
        .routes(routes!(verify_handler))
        .routes(routes!(auth_mode_handler))
        .routes(routes!(password_login_handler))
        .routes(routes!(change_password_handler))
        .routes(routes!(admin_reset_password_handler))
        .routes(routes!(me_handler, update_me_handler))
        .routes(routes!(logout_handler))
        .routes(routes!(logout_all_handler))
        .routes(routes!(exchange_api_key_handler))
        .routes(routes!(exchange_token_handler))
        .routes(routes!(create_api_key_handler, list_api_keys_handler))
        .routes(routes!(revoke_api_key_handler))
        .routes(routes!(create_pairing_code_handler))
        .routes(routes!(list_users_handler, create_user_handler))
        .routes(routes!(update_user_handler))
        .routes(routes!(
            set_user_provider_id_handler,
            delete_user_provider_id_handler
        ))
        .routes(routes!(list_user_provider_ids_handler))
}

// ============================================================================
// Local email/password authentication
// ============================================================================

/// Minimum length accepted for any password set through the API.
const MIN_PASSWORD_LEN: usize = 12;

/// Reject passwords that are too short to be worth hashing.
fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AuthError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

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

/// 404 unless this deployment runs local password auth or explicitly enables
/// the unadvertised break-glass credential.
fn require_local_auth(state: &AuthState) -> Result<(), AuthError> {
    if state.config.auth.provider == AuthProvider::Local || state.config.auth.break_glass_enabled {
        Ok(())
    } else {
        Err(AuthError::NotFound(
            "password authentication is not enabled".into(),
        ))
    }
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
async fn auth_mode_handler(State(state): State<AuthState>) -> Json<AuthModeResponse> {
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
async fn password_login_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AuthError> {
    require_local_auth(&state)?;

    let email = req.email.trim().to_lowercase();
    let source = request_source(&headers);
    let email_locked = state
        .login_throttle
        .is_locked("password_email", &email)
        .await?;
    let source_locked = match source {
        Some(source) => {
            state
                .login_throttle
                .is_locked("password_source", source)
                .await?
        }
        None => false,
    };
    if email_locked || source_locked {
        warn!("Password login rejected: throttled");
        return Err(AuthError::TooManyRequests);
    }

    let credential = state.credentials.find_by_email(&email).await?;

    // An unknown email must cost the same as a wrong password, and answer
    // identically, or the endpoint becomes an account-existence oracle.
    let Some(credential) = credential else {
        hash::verify_dummy(req.password).await;
        record_login_failure(&state, &email, source).await?;
        return Err(AuthError::Unauthorized);
    };

    let matches = hash::verify_password(req.password, credential.password_hash.clone())
        .await
        .map_err(AuthError::PasswordVerification)?;

    if !matches {
        record_login_failure(&state, &email, source).await?;
        if let Err(e) = state
            .credentials
            .record_login_failure(&credential.user_id)
            .await
        {
            warn!(error = %e, "Failed to record login failure");
        }
        return Err(AuthError::Unauthorized);
    }

    let user = state
        .users
        .get_user(&credential.user_id)
        .await?
        .ok_or_else(|| {
            // A credential without its user row means the FK was subverted.
            AuthError::Internal(format!(
                "credential {} has no matching users row",
                credential.user_id
            ))
        })?;

    revoke_present_session(&state, &headers).await?;
    let issued = session::create_session_from_user_with_provider(
        state.sessions.as_ref(),
        &user,
        None,
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
        "local",
    )
    .await?;

    state.login_throttle.clear("password_email", &email).await?;
    if let Err(e) = state
        .credentials
        .record_login_success(&credential.user_id, chrono::Utc::now())
        .await
    {
        warn!(error = %e, "Failed to record login success");
    }

    let resp_headers = issued_session_headers(&state, &issued);

    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.auth.session_absolute_age_secs as i64);

    info!(user_id = %user.id, login = %user.login, "Password login succeeded");

    Ok((
        resp_headers,
        Json(SessionResponse {
            user: Some(UserInfo {
                id: user.id,
                login: user.login,
                email: user.email,
                name: user.name,
                avatar_url: user.avatar_url,
                provider: "local".to_string(),
                role: user.role,
            }),
            expires_at: Some(format_datetime(expires_at)),
            providers: Vec::new(),
            setup: None,
        }),
    )
        .into_response())
}

async fn record_login_failure(
    state: &AuthState,
    email: &str,
    source: Option<&str>,
) -> Result<(), AuthError> {
    state
        .login_throttle
        .record_failure("password_email", email)
        .await?;
    if let Some(source) = source {
        state
            .login_throttle
            .record_failure("password_source", source)
            .await?;
    }
    Ok(())
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
async fn change_password_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    require_local_auth(&state)?;
    // Cookie only: an API key must not be escalatable into account takeover.
    let claims = authenticate_from_cookie(&state, &headers).await?;
    validate_password(&req.new_password)?;

    let credential = state
        .credentials
        .find_by_user_id(&claims.sub)
        .await?
        .ok_or(AuthError::Unauthorized)?;

    // Re-verify even though the caller is authenticated — a borrowed session
    // must not be enough to lock the owner out.
    let matches = hash::verify_password(req.current_password, credential.password_hash)
        .await
        .map_err(AuthError::PasswordVerification)?;
    if !matches {
        return Err(AuthError::Unauthorized);
    }

    let new_hash = hash::hash_password(req.new_password)
        .await
        .map_err(AuthError::PasswordHashing)?;
    state
        .credentials
        .set_password_hash(&claims.sub, &new_hash)
        .await?;

    // Invalidate every session, including this one — the browser re-logs in.
    if let Err(e) = state
        .sessions
        .delete_sessions_for_user(&claims.sub.to_string())
        .await
    {
        warn!(error = %e, "Failed to clear sessions after password change");
    }

    info!(user_id = %claims.sub, "Password changed");
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
async fn admin_reset_password_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
    Json(req): Json<AdminResetPasswordRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    require_local_auth(&state)?;
    let identity = authenticate_request(&state, &headers).await?;
    if identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }
    validate_password(&req.new_password)?;

    let user = state
        .users
        .get_user_by_login(&login)
        .await?
        .ok_or_else(|| AuthError::NotFound(format!("user '{login}' not found")))?;
    let credential = state
        .credentials
        .find_by_user_id(&user.id)
        .await?
        .ok_or_else(|| AuthError::NotFound(format!("no local credential for '{login}'")))?;

    let new_hash = hash::hash_password(req.new_password)
        .await
        .map_err(AuthError::PasswordHashing)?;

    // `upsert` carries `must_change`; `set_password_hash` always clears it.
    state
        .credentials
        .upsert(
            &credential.user_id,
            &credential.username,
            &new_hash,
            req.must_change_password.unwrap_or(false),
        )
        .await?;

    if let Err(e) = state
        .sessions
        .delete_sessions_for_user(&credential.user_id.to_string())
        .await
    {
        warn!(error = %e, "Failed to clear sessions after admin password reset");
    }

    info!(
        admin = %identity.user_id,
        target = %credential.user_id,
        "Admin reset a user password"
    );
    Ok(Json(MessageResponse {
        message: "Password reset successfully".to_string(),
    }))
}

/// Create a human user with a local password (the `isBot: false` branch of
/// `POST /api/v1/users`).
async fn create_local_user(
    state: &AuthState,
    req: CreateUserRequest,
) -> Result<(StatusCode, Json<CreateUserResponse>), AuthError> {
    if state.config.auth.provider != AuthProvider::Local {
        return Err(AuthError::BadRequest(
            "human password users can only be created in local auth mode".into(),
        ));
    }
    require_local_auth(state).map_err(|_| {
        AuthError::BadRequest(
            "human users can only be created when the server runs local password auth".into(),
        )
    })?;

    let login = req
        .login
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| AuthError::BadRequest("login is required when isBot is false".into()))?;
    let email = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|email| email.contains('@'))
        .ok_or_else(|| AuthError::BadRequest("valid email is required when isBot is false".into()))?
        .to_lowercase();
    let password = req
        .password
        .ok_or_else(|| AuthError::BadRequest("password is required when isBot is false".into()))?;
    validate_password(&password)?;

    if state.credentials.find_by_email(&email).await?.is_some()
        || state.users.find_user_by_email(&email).await?.is_some()
    {
        return Err(AuthError::Conflict(format!(
            "a user with email '{email}' already exists"
        )));
    }

    let role = req.role.unwrap_or(UserRole::User);
    // The internal id is a uuid; the `local:<login>` handle is preserved as the
    // provider identity so it resolves back to this uuid.
    let provider_user_id = format!("local:{email}");
    let user_id = uuid::Uuid::new_v4();

    // Hash before opening the transaction — argon2 costs ~100ms and must not
    // hold a Postgres connection open for it.
    let password_hash = hash::hash_password(password)
        .await
        .map_err(AuthError::PasswordHashing)?;

    let now = chrono::Utc::now();
    let mut tx = state.db.begin().await.map_err(AuthError::Database)?;

    sqlx::query(
        r#"
        INSERT INTO users (id, login, email, name, avatar_url, provider, provider_user_id, role, created_at, last_login_at, is_bot)
        VALUES ($1, $2, $3, $4, NULL, 'local', $5, $6, $7, $7, false)
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(user_id)
    .bind(login)
    .bind(&email)
    .bind(login)
    .bind(&provider_user_id)
    .bind(role.as_str())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id, created_at)
           VALUES ($1, 'local', $2, NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(user_id)
    .bind(&provider_user_id)
    .execute(&mut *tx)
    .await?;

    let inserted = sqlx::query(
        r#"INSERT INTO user_credentials (user_id, username, password_hash, must_change)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(user_id)
    .bind(&email)
    .bind(&password_hash)
    .bind(req.must_change_password.unwrap_or(false))
    .execute(&mut *tx)
    .await?;

    // A racing request won the unique index — surface 409, not a 500.
    if inserted.rows_affected() == 0 {
        return Err(AuthError::Conflict(format!(
            "a user with login '{login}' already exists"
        )));
    }

    tx.commit().await.map_err(AuthError::Database)?;

    info!(user_id = %user_id, login = %login, "Local user created");
    Ok((
        StatusCode::CREATED,
        Json(CreateUserResponse {
            id: user_id,
            login: login.to_string(),
            email: Some(email),
            is_bot: false,
            provider: "local".to_string(),
            role: role.to_string(),
        }),
    ))
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
    task_access: &dyn crate::extractors::TaskAccess,
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
async fn me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthError> {
    // Try API key first, fall back to session cookie
    let identity = authenticate_request(&state, &headers).await?;

    // Fetch fresh user data from DB (email/name/avatar may have changed)
    let user = state.users.get_user(&identity.user_id).await?;
    let Some(user) = user else {
        return Err(AuthError::SessionExpired);
    };

    let setup_ctx = SetupContext {
        user_id: &identity.user_id.to_string(),
        is_admin: identity.role == UserRole::Admin,
        provider_tokens: state.provider_tokens_dao.as_ref(),
        secrets: Some(state.secrets.as_ref()),
    };
    let user_id_str = identity.user_id.to_string();
    let (providers, setup) = tokio::join!(
        check_provider_connections(&state, &user_id_str),
        evaluate_setup(&setup_ctx),
    );

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
        setup: Some(setup),
    }))
}

/// Inspect external provider tokens for the current user and report whether
/// each provider is currently usable. Returns a best-effort list: DB errors
/// are logged but surfaced as `connected: false` rather than failing `/me`.
///
/// GitHub is always present; Codex appears only for users who connected it.
async fn check_provider_connections(state: &AuthState, user_id: &str) -> Vec<ProviderConnection> {
    let mut out = Vec::with_capacity(2);
    if let Some(github) = check_github_connection(state, user_id).await {
        out.push(github);
    }
    if let Some(codex) = check_codex_connection(state, user_id).await {
        out.push(codex);
    }
    out
}

/// Report the user's personal Codex credential in the `/me` providers array.
///
/// Returns `None` when the user has no codex row at all — most users never
/// connect Codex, and an absent entry keeps the frontend's `?? true` default
/// from showing a reconnect banner to them. When a row exists, `connected`
/// is false iff the refresher stamped `metadata.refresh_failure` (permanent
/// refresh failure — the user must sign in again).
async fn check_codex_connection(state: &AuthState, user_id: &str) -> Option<ProviderConnection> {
    let token = match state
        .provider_tokens_dao
        .find_provider_token_full(user_id, "codex")
        .await
    {
        Ok(row) => row?,
        Err(e) => {
            warn!(
                error = %e,
                user_id = %user_id,
                "Failed to load Codex provider token while building /me response"
            );
            return None;
        }
    };

    let metadata = token.metadata.as_ref();
    let needs_reconnect = metadata.is_some_and(|m| m.get("refresh_failure").is_some());
    let login = metadata
        .and_then(|m| m.get("email"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    Some(ProviderConnection {
        provider: "codex".to_string(),
        connected: !needs_reconnect,
        login,
    })
}

/// Returns `None` when the user has no GitHub token row at all — a user who
/// never connected GitHub is not "disconnected", and reporting them as such
/// pins a reconnect banner in the UI forever. Mirrors
/// [`check_codex_connection`].
async fn check_github_connection(state: &AuthState, user_id: &str) -> Option<ProviderConnection> {
    let disconnected = |login: Option<String>| {
        Some(ProviderConnection {
            provider: "github".to_string(),
            connected: false,
            login,
        })
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

    // No row → never connected, so report nothing rather than "disconnected".
    let token = token?;

    let login = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("login"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // When a GitHub App service is configured, verify the token by actually
    // resolving it. Any error (missing row, refresh rejected, dangling secret
    // reference) is reported as disconnected so the frontend prompts for a
    // reconnect.
    if state.github_app_service.is_some() {
        match state
            .git_host_tokens
            .github_is_usable(&moatless_types::access_control::Actor::of(user_id))
            .await
        {
            Ok(()) => Some(ProviderConnection {
                provider: "github".to_string(),
                connected: true,
                login,
            }),
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
        Some(ProviderConnection {
            provider: "github".to_string(),
            connected: true,
            login,
        })
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

async fn login_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
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
async fn cli_login_handler(
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

    // GitHub App login uses the stable `github` identity key shared with
    // repository credentials.
    let provider_str = provider_id.to_string();
    let provider_key = if provider_str == "github_app" {
        "github"
    } else {
        provider_str.as_str()
    };
    let user_id = match state
        .users
        .find_user_by_provider_id(provider_key, &user_info.id)
        .await
        .map_err(AuthError::Database)?
    {
        Some(user) => user.id,
        None => uuid::Uuid::new_v4(),
    };

    // Every login mechanism converges on the opaque auth-session issuer.
    revoke_present_session(state, headers).await?;
    let issued = session::create_session(
        state.users.as_ref(),
        state.sessions.as_ref(),
        state.provider_tokens.as_ref(),
        Some(state.secrets.as_ref()),
        user_id,
        &provider_str,
        &tokens,
        &user_info,
        group_id.as_deref(),
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
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
            &user_id.to_string(),
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
// OpenID Connect
// ============================================================================

#[derive(serde::Deserialize)]
struct OidcStartQuery {
    return_to: Option<String>,
    port: Option<i32>,
}

async fn oidc_start_handler(
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

async fn oidc_callback_handler(
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
            query.code,
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

/// Revoke every auth session belonging to the current user.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    tag = "auth",
    responses((status = 200, description = "All sessions revoked", body = MessageResponse))
)]
async fn logout_all_handler(
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
async fn exchange_token_handler(
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
            crate::extractors::TokenVerifyError::NotConfigured => {
                AuthError::Config("TASK_TOKEN_SECRET not configured".to_string())
            }
            crate::extractors::TokenVerifyError::Invalid(_) => AuthError::Unauthorized,
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
async fn create_api_key_handler(
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
async fn create_pairing_code_handler(
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
async fn list_api_keys_handler(
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
async fn revoke_api_key_handler(
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
async fn create_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateUserResponse>), AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    if identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }
    if !req.is_bot {
        return create_local_user(&state, req).await;
    }
    let app_key = req.github_app_key.as_deref().ok_or_else(|| {
        AuthError::BadRequest("githubAppKey is required when isBot is true".into())
    })?;
    let app = state.github_apps.require(app_key).await?;
    let bot_identity = app
        .get_bot_identity()
        .await
        .map_err(|e| AuthError::BotIdentity(Box::new(e)))?;

    let now = chrono::Utc::now();
    // The App key is preserved as the bot's provider identity; the internal id
    // is a uuid, reused across re-runs so the endpoint stays idempotent.
    let provider_user_id = format!("bot:{app_key}");

    // Atomic: users + provider_tokens row succeed together. A failure of the
    // provider_tokens insert must not leave a half-bot row in `users` that
    // silently falls back to "user with no creds". Re-running the endpoint with
    // the same app key refreshes the bot's login/email from the GitHub App
    // metadata via the ON CONFLICT clauses.
    let mut tx = state.db.begin().await.map_err(AuthError::Database)?;

    let existing_bot: Option<(uuid::Uuid,)> =
        sqlx::query_as("SELECT id FROM users WHERE provider_user_id = $1")
            .bind(&provider_user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let user_id = existing_bot.map_or_else(uuid::Uuid::new_v4, |(id,)| id);
    let user_id_str = user_id.to_string();

    sqlx::query(
        r#"
        INSERT INTO users (id, login, email, name, avatar_url, provider, provider_user_id, role, created_at, last_login_at, is_bot)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (id) DO UPDATE SET
            login = EXCLUDED.login,
            email = COALESCE(users.email, EXCLUDED.email),
            name = EXCLUDED.name,
            avatar_url = EXCLUDED.avatar_url,
            provider_user_id = COALESCE(EXCLUDED.provider_user_id, users.provider_user_id),
            last_login_at = EXCLUDED.last_login_at,
            is_bot = EXCLUDED.is_bot
        "#,
    )
    .bind(user_id)
    .bind(&bot_identity.login)
    .bind(Some(&bot_identity.email))
    .bind(Some(&bot_identity.login))
    .bind::<Option<String>>(None)
    .bind("github_app")
    .bind(&provider_user_id)
    .bind(UserRole::User.as_str())
    .bind(now)
    .bind(now)
    .bind(true)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id, created_at)
           VALUES ($1, 'github_app', $2, NOW())
           ON CONFLICT (user_id, provider) DO NOTHING"#,
    )
    .bind(user_id)
    .bind(&provider_user_id)
    .execute(&mut *tx)
    .await?;

    // Bot's provider_tokens row. `provider_user_id` carries the App key so
    // the canonical resolver can pick the right App when minting installation
    // tokens. `access_token` holds the documented sentinel — any code path that
    // bypasses the resolver and reads the column directly hits a clear 401
    // instead of silently sending an empty Bearer token.
    // team_id is NULL (migration 0097 dropped the team FK requirement).
    sqlx::query(
        r#"
        INSERT INTO provider_tokens (
            user_id, provider, auth_method, access_token, refresh_token,
            token_type, oauth_scope, provider_user_id, provider_login,
            connected_at, expires_at, scope, owner_user_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'user', $1)
        ON CONFLICT (user_id, provider, team_id) DO UPDATE SET
            auth_method = EXCLUDED.auth_method,
            access_token = EXCLUDED.access_token,
            refresh_token = EXCLUDED.refresh_token,
            token_type = EXCLUDED.token_type,
            oauth_scope = EXCLUDED.oauth_scope,
            scope = EXCLUDED.scope,
            owner_user_id = EXCLUDED.owner_user_id,
            provider_user_id = COALESCE(EXCLUDED.provider_user_id, provider_tokens.provider_user_id),
            provider_login = COALESCE(EXCLUDED.provider_login, provider_tokens.provider_login),
            connected_at = EXCLUDED.connected_at,
            expires_at = EXCLUDED.expires_at
        "#,
    )
    .bind(&user_id_str)
    .bind("github")
    .bind(super::provider_tokens::AUTH_METHOD_APP_INSTALLATION)
    .bind(super::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER)
    .bind::<Option<String>>(None) // refresh_token
    .bind::<Option<String>>(None) // token_type
    .bind::<Option<String>>(None) // oauth_scope
    .bind(Some(app_key))
    .bind(Some(&bot_identity.login))
    .bind(now)
    .bind::<Option<chrono::DateTime<chrono::Utc>>>(None)
    .execute(&mut *tx)
    .await?;

    tx.commit().await.map_err(AuthError::Database)?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateUserResponse {
            id: user_id,
            login: bot_identity.login,
            email: Some(bot_identity.email),
            is_bot: true,
            provider: "github_app".to_string(),
            role: "user".to_string(),
        }),
    ))
}

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
        })
        .collect();

    Ok(Json(UserListResponse { users: items }))
}

/// Update a user's profile (name and/or email).
///
/// A user may update their own profile; updating another user requires the
/// admin role. Backs the `moat user <login> set-email` CLI command so emails
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
async fn update_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
    Json(body): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserInfo>, AuthError> {
    if matches!(body.email, Some(None)) {
        return Err(AuthError::BadRequest(
            "human users must retain an email address".into(),
        ));
    }
    let identity = authenticate_request(&state, &headers).await?;
    let user_id = resolve_login_to_user_id(&state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }
    if body.role.is_some() && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    let user = state
        .users
        .update_user_profile_and_role(
            &user_id,
            body.name.clone(),
            body.email.clone(),
            body.role,
        )
        .await?
        .ok_or_else(|| AuthError::NotFound(format!("User '{login}' not found")))?;

    info!(
        actor = %identity.user_id,
        target_user = %user_id,
        "Updated user profile (name/email)"
    );

    Ok(Json(UserInfo {
        id: user.id,
        login: user.login,
        email: user.email,
        name: user.name,
        avatar_url: user.avatar_url,
        provider: user.provider,
        role: user.role,
    }))
}

// ============================================================================
// Provider ID management
// ============================================================================

/// Resolve a login to a user ID, returning 404 if not found.
async fn resolve_login_to_user_id(state: &AuthState, login: &str) -> Result<uuid::Uuid, AuthError> {
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
// Linear workspace OAuth install
// ============================================================================

/// Initiate the Linear workspace install flow.
///
/// Returns 302 to Linear's authorization URL with `actor=app` and the
/// workspace agent scopes. Sets an `oauth_state` cookie for CSRF.
/// Returns 503 when Linear OAuth credentials aren't configured.
async fn linear_install_handler(State(state): State<AuthState>) -> Result<Response, AuthError> {
    let Some(provider) = state.providers.linear.as_ref() else {
        return Err(AuthError::Config(
            "Linear OAuth not configured (set LINEAR_OAUTH_CLIENT_ID, \
             LINEAR_OAUTH_CLIENT_SECRET, LINEAR_OAUTH_CALLBACK_URL)"
                .into(),
        ));
    };

    let csrf_state = generate_csrf_state();
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
async fn linear_callback_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<super::schemas::OAuthCallbackQuery>,
) -> Result<Response, AuthError> {
    let provider = state
        .providers
        .linear
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

    let tokens = provider.exchange_code_for_tokens(&query.code).await?;
    let identity = provider
        .fetch_install_identity(&tokens.access_token)
        .await?;

    // Upsert the workspace-level adapter_connections row.
    use moatless_adapter_model::models::AdapterConnection;
    let conn_repo = &state.adapter_connections;

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
        adapter_kind: moatless_types::adapter::AdapterKind::Linear,
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
            .map_or(moatless_types::access_control::Scope::Global, |c| c.scope),
        owner_user_id: existing.as_ref().and_then(|c| c.owner_user_id.clone()),
        created_at: existing.as_ref().map_or(now, |c| c.created_at),
        updated_at: now,
    };
    // reason: OAuth install callback has no user session; the workspace
    // connection is a global admin-managed row.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = moatless_types::access_control::ScopeContext::system();
    conn_repo
        .upsert(&connection_row, &sys_ctx)
        .await
        .map_err(AuthError::Database)?;

    // Upsert the workspace token row (global scope: user_id=NULL, team_id=NULL).
    let expires_at = tokens
        .expires_in
        .map(|secs| now + chrono::Duration::seconds(secs as i64));
    let metadata = serde_json::json!({
        "organization_id": identity.organization_id,
        "organization_name": identity.organization_name,
        "agent_viewer_id": identity.agent_viewer_id,
        "agent_viewer_name": identity.agent_viewer_name,
        "agent_viewer_email": identity.agent_viewer_email,
        "scope": tokens.scope,
    });
    sqlx::query(
        r#"INSERT INTO provider_tokens (
               provider, auth_method, access_token, refresh_token, token_type,
               metadata, connected_at, updated_at, scope, owner_user_id, expires_at
           )
           VALUES ('linear', 'oauth', $1, $2, $3, $4, NOW(), NOW(), 'global', NULL, $5)
           ON CONFLICT (user_id, provider, team_id) DO UPDATE SET
               access_token = $1, refresh_token = $2, token_type = $3,
               metadata = $4, auth_method = 'oauth', scope = 'global',
               owner_user_id = NULL, updated_at = NOW(), expires_at = $5"#,
    )
    .bind(&tokens.access_token)
    .bind(tokens.refresh_token.as_deref())
    .bind(tokens.token_type.as_str())
    .bind(&metadata)
    .bind(expires_at)
    .execute(&state.db)
    .await
    .map_err(AuthError::Database)?;

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
    impl crate::extractors::TaskAccess for FakeTaskAccess {
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

    #[test]
    fn documented_auth_routes_can_be_constructed() {
        let _ = documented_auth_routes();
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
