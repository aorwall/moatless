//! The caller's own profile, and the admin routes over users and their provider
//! ids.

use super::session::{authenticate_from_cookie, authenticate_request};
use crate::auth::error::AuthError;
use crate::auth::schemas::{
    AdminUpdateUserRequest, CreateUserRequest, CreateUserResponse, ListUsersQuery, MessageResponse,
    SessionResponse, UpdateUserRequest, UserInfo, UserListItem, UserListResponse, format_datetime,
};
use crate::auth::state::AuthState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use moatless_api_schemas::error::ErrorBody;
use moatless_api_schemas::user::{SetProviderIdRequest, UserProviderIdItem};

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
pub(in crate::auth) async fn me_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, AuthError> {
    // Try API key first, fall back to session cookie
    let identity = authenticate_request(&state, &headers).await?;

    crate::auth::account_status::current_user(&state, identity)
        .await
        .map(Json)
}

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
pub(in crate::auth) async fn update_me_handler(
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

/// Create a user. Admin only.
///
/// `isBot: true` mints a `{slug}[bot]` identity plus a `provider_tokens` row so
/// credential resolution issues installation tokens for it; `isBot: false`
/// creates a local-password human user (local auth only).
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
pub(in crate::auth) async fn create_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(axum::http::StatusCode, Json<CreateUserResponse>), AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    let created = crate::auth::accounts::create_user(&state, &identity, req).await?;
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
pub(in crate::auth) async fn list_users_handler(
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

/// Update a user's profile (name and/or email). Only the fields present in the
/// body are changed.
///
/// A user may update their own profile; updating another user requires the
/// admin role.
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
pub(in crate::auth) async fn update_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
    Json(body): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserInfo>, AuthError> {
    crate::auth::accounts::validate_profile_update(&body)?;
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::update_user(&state, &identity, login, body).await?,
    ))
}

/// Update the user an internal id names. Admin only.
///
/// A login isn't unique — `PATCH /api/v1/users/{login}` resolves only the
/// oldest row of a shared login. Reach any other row here, keyed on the `id`
/// `GET /api/v1/users` reports; takes the same body.
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
pub(in crate::auth) async fn admin_update_user_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<uuid::Uuid>,
    Json(body): Json<AdminUpdateUserRequest>,
) -> Result<Json<UserInfo>, AuthError> {
    crate::auth::accounts::validate_profile_update(&body)?;
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::update_user_by_id(&state, &identity, user_id, body).await?,
    ))
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
pub(in crate::auth) async fn set_user_provider_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((login, provider)): Path<(String, String)>,
    Json(body): Json<SetProviderIdRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::set_user_provider_id(&state, &identity, login, provider, body)
            .await?,
    ))
}

/// Set (upsert) a provider-specific user ID, keyed on the internal user id.
///
/// A login isn't unique — `PUT /api/v1/users/{login}/provider-ids/{provider}`
/// resolves only the oldest row of a shared login. Reach any other row here,
/// keyed on the `id` `GET /api/v1/users` reports; same body, same policy.
#[utoipa::path(
    put,
    path = "/api/v1/users/by-id/{user_id}/provider-ids/{provider}",
    tag = "users",
    params(
        ("user_id" = uuid::Uuid, Path, description = "Target user id"),
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
pub(in crate::auth) async fn set_user_provider_id_by_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((user_id, provider)): Path<(uuid::Uuid, String)>,
    Json(body): Json<SetProviderIdRequest>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::set_user_provider_id_by_id(
            &state, &identity, user_id, provider, body,
        )
        .await?,
    ))
}

/// Delete a provider ID mapping, keyed on the internal user id.
///
/// See [`set_user_provider_id_by_id_handler`] for why a login is not enough.
#[utoipa::path(
    delete,
    path = "/api/v1/users/by-id/{user_id}/provider-ids/{provider}",
    tag = "users",
    params(
        ("user_id" = uuid::Uuid, Path, description = "Target user id"),
        ("provider" = String, Path, description = "Provider name, e.g. 'slack'")
    ),
    responses(
        (status = 200, description = "Provider ID deleted", body = MessageResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 403, description = "Access denied", body = ErrorBody),
        (status = 404, description = "User or mapping not found", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn delete_user_provider_id_by_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((user_id, provider)): Path<(uuid::Uuid, String)>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::delete_user_provider_id_by_id(&state, &identity, user_id, provider)
            .await?,
    ))
}

/// List all provider ID mappings, keyed on the internal user id.
///
/// See [`set_user_provider_id_by_id_handler`] for why a login is not enough.
#[utoipa::path(
    get,
    path = "/api/v1/users/by-id/{user_id}/provider-ids",
    tag = "users",
    params(("user_id" = uuid::Uuid, Path, description = "Target user id")),
    responses(
        (status = 200, description = "List of provider ID mappings", body = Vec<UserProviderIdItem>),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn list_user_provider_ids_by_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(user_id): Path<uuid::Uuid>,
) -> Result<Json<Vec<UserProviderIdItem>>, AuthError> {
    authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::list_user_provider_ids_by_id(&state, user_id).await?,
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
pub(in crate::auth) async fn delete_user_provider_id_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path((login, provider)): Path<(String, String)>,
) -> Result<Json<MessageResponse>, AuthError> {
    let identity = authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::delete_user_provider_id(&state, &identity, login, provider).await?,
    ))
}

/// List all provider ID mappings for a moatless user.
///
/// Any authenticated caller may read these; writing one is still self-or-admin.
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
        (status = 404, description = "User not found", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn list_user_provider_ids_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(login): Path<String>,
) -> Result<Json<Vec<UserProviderIdItem>>, AuthError> {
    authenticate_request(&state, &headers).await?;
    Ok(Json(
        crate::auth::accounts::list_user_provider_ids(&state, login).await?,
    ))
}
