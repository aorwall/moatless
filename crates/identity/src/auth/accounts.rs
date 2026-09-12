//! Account creation policy and password validation.

use super::schemas::{AdminUpdateUserRequest, MessageResponse, UserInfo, format_datetime};
use super::{
    error::AuthError,
    password::hash,
    schemas::{CreateUserRequest, CreateUserResponse},
    state::AuthState,
};
use moatless_api_schemas::user::{SetProviderIdRequest, UserProviderIdItem};
use moatless_types::{
    auth::{AuthIdentity, AuthProvider},
    user::{UserRole, UserStatus},
};
use tracing::info;

/// Minimum length accepted for any password set through the API.
const MIN_PASSWORD_LEN: usize = 12;

/// Reject passwords that are too short to be worth hashing.
pub(super) fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AuthError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

pub(super) fn require_local_auth(state: &AuthState) -> Result<(), AuthError> {
    if state.config.auth.provider == AuthProvider::Local || state.config.auth.break_glass_enabled {
        Ok(())
    } else {
        Err(AuthError::NotFound(
            "password authentication is not enabled".into(),
        ))
    }
}

async fn create_local_user(
    state: &AuthState,
    req: CreateUserRequest,
) -> Result<CreateUserResponse, AuthError> {
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

    super::account_dao::insert_local(
        &state.db,
        super::account_dao::LocalAccount {
            user_id,
            login,
            email: &email,
            provider_user_id: &provider_user_id,
            role,
            password_hash: &password_hash,
            must_change: req.must_change_password.unwrap_or(false),
        },
    )
    .await?;

    info!(user_id = %user_id, login = %login, "Local user created");
    Ok(CreateUserResponse {
        id: user_id,
        login: login.to_string(),
        email: Some(email),
        is_bot: false,
        provider: "local".to_string(),
        role: role.to_string(),
    })
}

/// Create a local user or refresh a bot account for an administrator.
pub(super) async fn create_user(
    state: &AuthState,
    identity: &AuthIdentity,
    req: CreateUserRequest,
) -> Result<CreateUserResponse, AuthError> {
    if identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }
    if !req.is_bot {
        return create_local_user(state, req).await;
    }
    let app_key = req.github_app_key.as_deref().ok_or_else(|| {
        AuthError::BadRequest("githubAppKey is required when isBot is true".into())
    })?;
    let app = state.github_apps.require(app_key).await?;
    let bot_identity = app
        .get_bot_identity()
        .await
        .map_err(|e| AuthError::BotIdentity(Box::new(e)))?;

    let user_id = super::account_dao::upsert_bot(&state.db, app_key, &bot_identity).await?;

    Ok(CreateUserResponse {
        id: user_id,
        login: bot_identity.login,
        email: Some(bot_identity.email),
        is_bot: true,
        provider: "github_app".to_string(),
        role: "user".to_string(),
    })
}

pub(super) fn validate_profile_update(body: &AdminUpdateUserRequest) -> Result<(), AuthError> {
    if matches!(body.email, Some(None)) {
        return Err(AuthError::BadRequest(
            "human users must retain an email address".into(),
        ));
    }
    Ok(())
}

/// Set user provider id under the authenticated account policy.
pub(super) async fn set_user_provider_id(
    state: &AuthState,
    identity: &AuthIdentity,
    login: String,
    provider: String,
    body: SetProviderIdRequest,
) -> Result<MessageResponse, AuthError> {
    let user_id = resolve_login_to_user_id(state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    state
        .users
        .upsert_provider_id(&user_id, &provider, &body.provider_user_id)
        .await?;

    Ok(MessageResponse {
        message: format!("Provider ID for '{provider}' set successfully"),
    })
}

/// Delete user provider id under the authenticated account policy.
pub(super) async fn delete_user_provider_id(
    state: &AuthState,
    identity: &AuthIdentity,
    login: String,
    provider: String,
) -> Result<MessageResponse, AuthError> {
    let user_id = resolve_login_to_user_id(state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    let deleted = state.users.delete_provider_id(&user_id, &provider).await?;
    if !deleted {
        return Err(AuthError::NotFound(format!(
            "No '{provider}' provider ID found for user '{login}'"
        )));
    }

    Ok(MessageResponse {
        message: format!("Provider ID for '{provider}' deleted"),
    })
}

/// List user provider ids under the authenticated account policy.
pub(super) async fn list_user_provider_ids(
    state: &AuthState,
    identity: &AuthIdentity,
    login: String,
) -> Result<Vec<UserProviderIdItem>, AuthError> {
    let user_id = resolve_login_to_user_id(state, &login).await?;

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

    Ok(items)
}

/// Update user under the authenticated account policy.
pub(super) async fn update_user(
    state: &AuthState,
    identity: &AuthIdentity,
    login: String,
    body: AdminUpdateUserRequest,
) -> Result<UserInfo, AuthError> {
    validate_profile_update(&body)?;
    let user_id = resolve_login_to_user_id(state, &login).await?;

    if identity.user_id != user_id && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }
    if (body.role.is_some() || body.status.is_some()) && identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    apply_user_update(state, identity, user_id, body).await
}

/// Update the user `user_id` names, for an administrator.
///
/// The only handle on a row that [`resolve_login_to_user_id`] passes over.
pub(super) async fn update_user_by_id(
    state: &AuthState,
    identity: &AuthIdentity,
    user_id: uuid::Uuid,
    body: AdminUpdateUserRequest,
) -> Result<UserInfo, AuthError> {
    validate_profile_update(&body)?;
    if identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }

    apply_user_update(state, identity, user_id, body).await
}

/// Write an authorized update and report the row it left behind.
async fn apply_user_update(
    state: &AuthState,
    identity: &AuthIdentity,
    user_id: uuid::Uuid,
    body: AdminUpdateUserRequest,
) -> Result<UserInfo, AuthError> {
    // Disabling yourself refuses every credential you hold, and re-enabling
    // needs an admin session. There is no way back inside the product.
    if body.status == Some(UserStatus::Disabled) && user_id == identity.user_id {
        return Err(AuthError::BadRequest(
            "an admin cannot disable their own account".into(),
        ));
    }

    let user = state
        .users
        .update_user_profile_and_role(
            &user_id,
            body.name.clone(),
            body.email.clone(),
            body.role,
            body.status,
        )
        .await?
        .ok_or_else(|| AuthError::NotFound(format!("User '{user_id}' not found")))?;

    info!(
        actor = %identity.user_id,
        target_user = %user_id,
        name = body.name.is_some(),
        email = body.email.is_some(),
        role = ?body.role,
        status = ?body.status,
        "Updated user"
    );

    Ok(UserInfo {
        id: user.id,
        login: user.login,
        email: user.email,
        name: user.name,
        avatar_url: user.avatar_url,
        provider: user.provider,
        role: user.role,
    })
}

async fn resolve_login_to_user_id(state: &AuthState, login: &str) -> Result<uuid::Uuid, AuthError> {
    let user = state
        .users
        .get_user_by_login(login)
        .await?
        .ok_or_else(|| AuthError::NotFound(format!("User '{login}' not found")))?;
    Ok(user.id)
}
