use super::{
    accounts::{require_local_auth, validate_password},
    error::AuthError,
    password::hash,
    schemas::{
        AdminResetPasswordRequest, ChangePasswordRequest, PasswordLoginRequest, SessionResponse,
        UserInfo, format_datetime,
    },
    sessions::{self as session, IssuedAuthSession},
    state::AuthState,
};
use moatless_types::{auth::AuthIdentity, user::UserRole};
use tracing::{info, warn};

pub(super) async fn login(
    state: &AuthState,
    req: PasswordLoginRequest,
    source: Option<&str>,
    present_token: Option<&str>,
) -> Result<(IssuedAuthSession, SessionResponse), AuthError> {
    require_local_auth(state)?;

    let email = req.email.trim().to_lowercase();
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
        record_login_failure(state, &email, source).await?;
        return Err(AuthError::Unauthorized);
    };

    let matches = hash::verify_password(req.password, credential.password_hash.clone())
        .await
        .map_err(AuthError::PasswordVerification)?;

    if !matches {
        record_login_failure(state, &email, source).await?;
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

    if let Some(token) = present_token {
        session::revoke_session(state.sessions.as_ref(), token).await?;
    }
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

    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(state.config.auth.session_absolute_age_secs as i64);

    info!(user_id = %user.id, login = %user.login, "Password login succeeded");

    Ok((
        issued,
        SessionResponse {
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
        },
    ))
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

pub(super) async fn change_password(
    state: &AuthState,
    user_id: uuid::Uuid,
    req: ChangePasswordRequest,
) -> Result<(), AuthError> {
    require_local_auth(state)?;
    validate_password(&req.new_password)?;

    let credential = state
        .credentials
        .find_by_user_id(&user_id)
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
        .set_password_hash(&user_id, &new_hash)
        .await?;

    // Invalidate every session, including this one — the browser re-logs in.
    if let Err(e) = state
        .sessions
        .delete_sessions_for_user(&user_id.to_string())
        .await
    {
        warn!(error = %e, "Failed to clear sessions after password change");
    }

    info!(user_id = %user_id, "Password changed");
    Ok(())
}

pub(super) async fn reset_password(
    state: &AuthState,
    identity: &AuthIdentity,
    login: &str,
    req: AdminResetPasswordRequest,
) -> Result<(), AuthError> {
    require_local_auth(state)?;
    if identity.role != UserRole::Admin {
        return Err(AuthError::Forbidden);
    }
    validate_password(&req.new_password)?;

    let user = state
        .users
        .get_user_by_login(login)
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
    Ok(())
}
