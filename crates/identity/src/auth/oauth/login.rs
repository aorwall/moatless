use super::provider::OAuthProvider;
use crate::auth::{
    api_key,
    error::AuthError,
    sessions::{self as session, IssuedAuthSession},
    state::AuthState,
};
use moatless_types::auth::AuthProvider;
use tracing::warn;

pub(in crate::auth) enum LoginOutcome {
    OrganizationRequired,
    Completed {
        issued: IssuedAuthSession,
        login: String,
        cli_key: Option<String>,
    },
}

pub(in crate::auth) async fn complete_login(
    state: &AuthState,
    provider: &dyn OAuthProvider,
    provider_id: AuthProvider,
    code: Option<&str>,
    present_token: Option<&str>,
    cli: bool,
) -> Result<LoginOutcome, AuthError> {
    // A login has nowhere to report a decline to, so it stays an error.
    let code = code.ok_or_else(|| AuthError::OAuth("Authorization was declined".into()))?;

    let tokens = provider.exchange_code_for_tokens(code).await?;

    let user_info = provider.get_user_info(&tokens).await?;

    // Check required GitHub org membership. Login-only: a connect (see
    // `github_connect::finish`) does not gate on this, because it never
    // decides who the caller is.
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
            return Ok(LoginOutcome::OrganizationRequired);
        }
    }

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
    if let Some(token) = present_token {
        session::revoke_session(state.sessions.as_ref(), token).await?;
    }
    let issued = session::create_session(
        state.users.as_ref(),
        state.sessions.as_ref(),
        state.provider_tokens.as_ref(),
        state.secrets.as_ref(),
        user_id,
        &provider_str,
        &tokens,
        &user_info,
        group_id.as_deref(),
        state.config.auth.session_max_age_secs,
        state.config.auth.session_absolute_age_secs,
    )
    .await?;

    let cli_key = if cli {
        Some(
            api_key::generate_key(
                state.api_keys.as_ref(),
                &user_id.to_string(),
                "moat-cli",
                None,
                None,
                None,
            )
            .await?
            .key,
        )
    } else {
        None
    };
    Ok(LoginOutcome::Completed {
        issued,
        login: user_info.login,
        cli_key,
    })
}
